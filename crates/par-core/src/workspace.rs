use crate::frontend::lower;
use crate::frontend::parse_source_file;
use crate::frontend_impl::language::{
    BuiltinOperatorModule, CompileError, GlobalName, Resolved, ResolvedPackageRef, TypeParameter,
    Universal, Unresolved,
};
use crate::frontend_impl::parse::SyntaxError;
use crate::frontend_impl::process;
use crate::frontend_impl::program::{
    CheckedModule, DocComment, Docs, HoverIndex, ImportDecl, ImportPath, Module, SourceFile,
};
use crate::frontend_impl::types::display::{GlobalNameWriter, TypeRenderOptions};
use crate::frontend_impl::types::error::labels_from_span;
use crate::frontend_impl::types::{
    Type, TypeError, Visibility, VisibilityIndex, validate_visibility,
};
use crate::location::{FileName, Span, Spanning};
use crate::runtime_impl::{Compiled, RuntimeCompilerError};
use arcstr::ArcStr;
use indexmap::{IndexMap, IndexSet};
use par_runtime::linker::Unlinked;
use par_runtime::pkgid::{BuiltinPackage, PackageId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, btree_map::Entry};
use std::fmt::{self, Display, Formatter, Write};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod completion;

pub use completion::{CompletionCandidate, CompletionCandidateKind};

pub type ExternalModule = Module<Arc<process::Expression<(), Unresolved>>, Unresolved>;

const MANIFEST_FILE: &str = "Par.toml";
const SOURCE_DIRECTORY: &str = "src";
pub const DEPENDENCIES_DIRECTORY: &str = "dependencies";

fn message_report(message: impl Into<String>) -> miette::Report {
    miette::miette!("{}", message.into())
}

fn report_with_source_span(
    source: Arc<str>,
    span: &Span,
    message: impl Into<String>,
) -> miette::Report {
    let labels = labels_from_span(&source, span);
    let code: Arc<str> = if labels.is_empty() {
        "<UI>".into()
    } else {
        source
    };
    miette::miette!(labels = labels, "{}", message.into()).with_source_code(code)
}

#[derive(Debug, Clone)]
pub struct PackageLayout {
    pub root_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub src_dir: PathBuf,
}

impl PackageLayout {
    pub fn find_from(start: impl AsRef<Path>) -> Result<Self, WorkspaceDiscoveryError> {
        let start = start.as_ref();
        let mut cursor = if start.is_dir() {
            start.to_path_buf()
        } else {
            start
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."))
        };
        let original_start = start.to_path_buf();

        loop {
            if let Some(layout) = Self::from_existing_root_dir(cursor.clone())? {
                return Ok(layout);
            }

            if !cursor.pop() {
                return Err(WorkspaceDiscoveryError::PackageRootNotFound {
                    start: original_start,
                });
            }
        }
    }

    fn from_existing_root_dir(root_dir: PathBuf) -> Result<Option<Self>, WorkspaceDiscoveryError> {
        let manifest_path = root_dir.join(MANIFEST_FILE);
        if !manifest_path.is_file() {
            return Ok(None);
        }
        if let Err(error) = fs::read_to_string(&manifest_path) {
            return Err(WorkspaceDiscoveryError::ManifestReadError {
                path: manifest_path,
                message: error.to_string(),
            });
        }

        let src_dir = root_dir.join(SOURCE_DIRECTORY);
        if !src_dir.is_dir() {
            return Err(WorkspaceDiscoveryError::SrcDirectoryMissing { path: src_dir });
        }

        Ok(Some(Self {
            root_dir,
            manifest_path,
            src_dir,
        }))
    }

    fn find_existing_dependency_path(
        dependency_path: &Path,
    ) -> Result<Option<Self>, WorkspaceDiscoveryError> {
        if !dependency_path.exists() {
            return Ok(None);
        }
        let root_dir = if dependency_path.is_dir() {
            dependency_path.to_path_buf()
        } else {
            dependency_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| dependency_path.to_path_buf())
        };
        Self::from_existing_root_dir(root_dir)
    }

    fn from_dependency_path(
        dependency_path: &Path,
        declaring_manifest_path: &Path,
        alias: &str,
    ) -> Result<Self, WorkspaceDiscoveryError> {
        match Self::find_existing_dependency_path(dependency_path)? {
            Some(layout) => Ok(layout),
            None => Err(WorkspaceDiscoveryError::MissingDependencyPackage {
                manifest_path: declaring_manifest_path.to_path_buf(),
                alias: alias.to_string(),
                path: dependency_path.to_path_buf(),
            }),
        }
    }

    fn resolve_local_dependency(
        &self,
        raw_path: &Path,
        alias: &str,
    ) -> Result<PathBuf, WorkspaceDiscoveryError> {
        let raw = raw_path.to_string_lossy();
        let raw_owned = raw.to_string();

        if raw.starts_with('.') {
            return Ok(self.root_dir.join(raw_path));
        }

        if raw == "~" || raw.starts_with("~/") {
            let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
            let Some(home) = home else {
                return Err(WorkspaceDiscoveryError::InvalidDependencyPath {
                    manifest_path: self.manifest_path.clone(),
                    alias: alias.to_string(),
                    path: raw_owned,
                    message: String::from("HOME is not set"),
                });
            };
            let mut resolved = PathBuf::from(home);
            if let Some(tail) = raw.strip_prefix("~/") {
                resolved.push(tail);
            }
            return Ok(resolved);
        }

        if let Some(rest) = raw.strip_prefix('$') {
            let (variable, tail) = match rest.split_once('/') {
                Some((variable, tail)) => (variable, Some(tail)),
                None => (rest, None),
            };
            if variable.is_empty() {
                return Err(WorkspaceDiscoveryError::InvalidDependencyPath {
                    manifest_path: self.manifest_path.clone(),
                    alias: alias.to_string(),
                    path: raw_owned,
                    message: String::from("missing environment variable name after `$`"),
                });
            }
            let Some(value) = std::env::var_os(variable) else {
                return Err(WorkspaceDiscoveryError::InvalidDependencyPath {
                    manifest_path: self.manifest_path.clone(),
                    alias: alias.to_string(),
                    path: raw_owned,
                    message: format!("environment variable `{variable}` is not set"),
                });
            };

            let mut resolved = PathBuf::from(value);
            if let Some(tail) = tail
                && !tail.is_empty()
            {
                resolved.push(tail);
            }
            return Ok(resolved);
        }

        Err(WorkspaceDiscoveryError::InvalidDependencyPath {
            manifest_path: self.manifest_path.clone(),
            alias: alias.to_string(),
            path: raw_owned,
            message: String::from("expected a local path starting with `.`, `~`, or `$`"),
        })
    }

    pub fn managed_remote_dependency_path(&self, source: &RemoteDependencySource) -> PathBuf {
        self.root_dir
            .join(DEPENDENCIES_DIRECTORY)
            .join(source.managed_subpath())
    }
}

pub type SourceOverrides = HashMap<PathBuf, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencySpec {
    Local(PathBuf),
    Remote(RemoteDependencySource),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RemoteDependencySource(String);

impl RemoteDependencySource {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let normalized = raw
            .trim()
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .to_owned();
        if normalized.is_empty() {
            return Err(String::from("dependency source is empty"));
        }
        if normalized.contains("://") {
            return Err(String::from(
                "expected a host/path source without a URL scheme",
            ));
        }

        for segment in normalized.split('/') {
            if segment.is_empty() {
                return Err(String::from("dependency path contains an empty segment"));
            }
            if matches!(segment, "." | "..") {
                return Err(String::from("dependency path cannot contain `.` or `..`"));
            }
            if segment
                .chars()
                .any(|ch| ch == '\\' || ch == ':' || ch.is_whitespace() || ch.is_control())
            {
                return Err(format!("dependency path segment `{segment}` is invalid"));
            }
        }

        Ok(Self(normalized))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn managed_subpath(&self) -> PathBuf {
        self.0.split('/').collect()
    }
}

impl Display for RemoteDependencySource {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageManifest {
    pub name: String,
    pub dependencies: BTreeMap<String, DependencySpec>,
    pub builtin: bool,
}

impl PackageManifest {
    pub fn read_from(manifest_path: &Path) -> Result<Self, WorkspaceDiscoveryError> {
        let source = fs::read_to_string(manifest_path).map_err(|error| {
            WorkspaceDiscoveryError::ManifestReadError {
                path: manifest_path.to_path_buf(),
                message: error.to_string(),
            }
        })?;
        let manifest = toml::from_str::<ManifestToml>(&source).map_err(|error| {
            WorkspaceDiscoveryError::ManifestParseError {
                path: manifest_path.to_path_buf(),
                message: error.to_string(),
            }
        })?;

        Ok(Self {
            name: manifest.package.name,
            builtin: manifest.package.__builtin,
            dependencies: manifest
                .dependencies
                .into_iter()
                .map(|(alias, dependency)| {
                    let spec = if dependency.starts_with('.')
                        || dependency.starts_with('~')
                        || dependency.starts_with('$')
                    {
                        Ok(DependencySpec::Local(PathBuf::from(dependency)))
                    } else {
                        RemoteDependencySource::parse(&dependency)
                            .map(DependencySpec::Remote)
                            .map_err(|message| WorkspaceDiscoveryError::ManifestParseError {
                                path: manifest_path.to_path_buf(),
                                message: format!("invalid dependency `{alias}`: {message}"),
                            })
                    }?;
                    Ok((alias, spec))
                })
                .collect::<Result<_, _>>()?,
        })
    }

    pub fn render(&self) -> String {
        let manifest = ManifestToml {
            package: ManifestPackageToml {
                name: self.name.clone(),
                __builtin: self.builtin,
            },
            dependencies: self
                .dependencies
                .iter()
                .map(|(alias, dependency)| {
                    let source = match dependency {
                        DependencySpec::Local(path) => normalized_package_path(path),
                        DependencySpec::Remote(source) => source.to_string(),
                    };
                    (alias.clone(), source)
                })
                .collect(),
        };
        toml::to_string(&manifest).expect("package manifest should serialize to TOML")
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ManifestToml {
    package: ManifestPackageToml,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    dependencies: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ManifestPackageToml {
    name: String,
    #[serde(default)]
    __builtin: bool,
}

#[derive(Debug, Clone)]
pub struct LoadedPackageFile {
    pub name: FileName,
    pub relative_path_from_src: PathBuf,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModulePath {
    pub directories: Vec<String>,
    pub module: String,
}

impl ModulePath {
    pub fn to_slash_path(&self) -> String {
        if self.directories.is_empty() {
            return self.module.clone();
        }
        format!("{}/{}", self.directories.join("/"), self.module)
    }

    fn key(&self) -> ModulePathKey {
        ModulePathKey {
            directories: self.directories.clone(),
            module_lower: self.module.to_lowercase(),
        }
    }
}

impl Display for ModulePath {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_slash_path())
    }
}

#[derive(Debug, Clone)]
pub struct ParsedPackageFile {
    pub name: FileName,
    pub relative_path_from_src: PathBuf,
    pub source: Arc<str>,
    pub module_part_suffix: Option<String>,
    pub source_file: SourceFile<crate::frontend_impl::language::Expression<Unresolved>>,
}

#[derive(Debug, Clone)]
pub struct ParsedModule {
    pub path: ModulePath,
    pub doc: Option<DocComment>,
    pub files: Vec<ParsedPackageFile>,
}

#[derive(Debug, Clone)]
pub struct ParsedPackage {
    pub modules: Vec<ParsedModule>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ModulePathKey {
    directories: Vec<String>,
    module_lower: String,
}

#[derive(Debug, Clone)]
pub enum PackageLoadError {
    DirectoryReadError {
        path: PathBuf,
        message: String,
    },
    FileReadError {
        path: PathBuf,
        message: String,
    },
    InvalidSourceFilePath {
        path: PathBuf,
    },
    InvalidSourceFileName {
        path: PathBuf,
    },
    ParseError {
        file: FileName,
        source: Arc<str>,
        error: SyntaxError,
    },
    MissingModuleDeclaration {
        file: FileName,
    },
    FileNameModuleMismatch {
        file: FileName,
        declared_module: String,
        file_module_name: String,
    },
    ConflictingModuleNameCasing {
        module_path: String,
        first_file: FileName,
        first_declared_name: String,
        second_file: FileName,
        second_declared_name: String,
    },
}

impl Display for PackageLoadError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParseError { .. } => write!(f, "{:?}", self.to_report()),
            Self::DirectoryReadError { path, message } => {
                write!(
                    f,
                    "Failed to read directory {}: {}",
                    path.display(),
                    message
                )
            }
            Self::FileReadError { path, message } => {
                write!(
                    f,
                    "Failed to read source file {}: {}",
                    path.display(),
                    message
                )
            }
            Self::InvalidSourceFilePath { path } => {
                write!(f, "Source file path is not valid UTF-8: {}", path.display())
            }
            Self::InvalidSourceFileName { path } => {
                write!(
                    f,
                    "Invalid source file name (expected `Module.par` or `Module.*.par`): {}",
                    path.display()
                )
            }
            Self::MissingModuleDeclaration { file } => {
                write!(f, "Source file is missing `module` declaration: {}", file.0)
            }
            Self::FileNameModuleMismatch {
                file,
                declared_module,
                file_module_name,
            } => {
                write!(
                    f,
                    "Module declaration `{}` does not match source file name `{}` in {}",
                    declared_module, file_module_name, file.0
                )
            }
            Self::ConflictingModuleNameCasing {
                module_path,
                first_file,
                first_declared_name,
                second_file,
                second_declared_name,
            } => {
                write!(
                    f,
                    "Module `{}` has inconsistent declaration casing (`{}` in {}, `{}` in {})",
                    module_path,
                    first_declared_name,
                    first_file.0,
                    second_declared_name,
                    second_file.0
                )
            }
        }
    }
}

impl PackageLoadError {
    pub fn spans(&self) -> (Span, Vec<Span>) {
        match self {
            Self::ParseError { error, .. } => (error.span(), vec![]),
            Self::DirectoryReadError { .. }
            | Self::FileReadError { .. }
            | Self::InvalidSourceFilePath { .. }
            | Self::InvalidSourceFileName { .. }
            | Self::MissingModuleDeclaration { .. }
            | Self::FileNameModuleMismatch { .. }
            | Self::ConflictingModuleNameCasing { .. } => (Span::None, vec![]),
        }
    }

    pub fn to_report(&self) -> miette::Report {
        match self {
            Self::ParseError { source, error, .. } => {
                miette::Report::from(error.to_owned()).with_source_code(source.clone())
            }
            Self::DirectoryReadError { .. }
            | Self::FileReadError { .. }
            | Self::InvalidSourceFilePath { .. }
            | Self::InvalidSourceFileName { .. }
            | Self::MissingModuleDeclaration { .. }
            | Self::FileNameModuleMismatch { .. }
            | Self::ConflictingModuleNameCasing { .. } => message_report(self.to_string()),
        }
    }
}

impl std::error::Error for PackageLoadError {}

#[derive(Debug, Clone)]
pub enum WorkspaceDiscoveryError {
    PackageRootNotFound {
        start: PathBuf,
    },
    ManifestReadError {
        path: PathBuf,
        message: String,
    },
    ManifestParseError {
        path: PathBuf,
        message: String,
    },
    SrcDirectoryMissing {
        path: PathBuf,
    },
    PathCanonicalizationError {
        path: PathBuf,
        message: String,
    },
    Load(PackageLoadError),
    MissingDependencyPackage {
        manifest_path: PathBuf,
        alias: String,
        path: PathBuf,
    },
    InvalidDependencyPath {
        manifest_path: PathBuf,
        alias: String,
        path: String,
        message: String,
    },
    InvalidRemoteDependencySource {
        manifest_path: PathBuf,
        alias: String,
        source: String,
        message: String,
    },
    MissingRemoteDependencyPackage {
        manifest_path: PathBuf,
        alias: String,
        dependency: String,
        path: PathBuf,
    },
    RemoteDependencyMaterializationError {
        manifest_path: PathBuf,
        alias: String,
        dependency: String,
        path: PathBuf,
        message: String,
    },
    DependencyAliasCollision {
        package: PackageId,
        alias: String,
    },
    DuplicatePackageId {
        package: PackageId,
        first_root: PathBuf,
        second_root: PathBuf,
    },
    PackageCycle {
        cycle: Vec<PathBuf>,
    },
    UnknownBuiltinPackage {
        name: String,
    },
    DirectDependencyOnBuiltin {
        package: PackageId,
    },
    BuiltinDependencyOnNonBuiltin {
        package: PackageId,
    },
}

impl Display for WorkspaceDiscoveryError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(error) => write!(f, "{error}"),
            Self::PackageRootNotFound { start } => {
                write!(
                    f,
                    "Could not find `{MANIFEST_FILE}` when searching from {}",
                    start.display()
                )
            }
            Self::ManifestReadError { path, message } => {
                write!(f, "Failed to read manifest {}: {}", path.display(), message)
            }
            Self::ManifestParseError { path, message } => {
                write!(
                    f,
                    "Failed to parse manifest {}: {}",
                    path.display(),
                    message
                )
            }
            Self::SrcDirectoryMissing { path } => {
                write!(
                    f,
                    "Package source directory does not exist: {}",
                    path.display()
                )
            }
            Self::PathCanonicalizationError { path, message } => {
                write!(
                    f,
                    "Failed to canonicalize package path {}: {}",
                    path.display(),
                    message
                )
            }
            Self::MissingDependencyPackage {
                manifest_path,
                alias,
                path,
            } => write!(
                f,
                "Manifest {} declares dependency `@{}` at {}, but no package was found there",
                manifest_path.display(),
                alias,
                path.display()
            ),
            Self::InvalidDependencyPath {
                manifest_path,
                alias,
                path,
                message,
            } => write!(
                f,
                "Manifest {} declares invalid local dependency path `{}` for alias `@{}`: {}",
                manifest_path.display(),
                path,
                alias,
                message
            ),
            Self::InvalidRemoteDependencySource {
                manifest_path,
                alias,
                source,
                message,
            } => write!(
                f,
                "Manifest {} declares invalid remote dependency `{}` for alias `@{}`: {}",
                manifest_path.display(),
                source,
                alias,
                message
            ),
            Self::MissingRemoteDependencyPackage {
                manifest_path,
                alias,
                dependency,
                path,
            } => write!(
                f,
                "Manifest {} declares remote dependency `{}` for alias `@{}`, but no package was found at {}. Run `par add` to fetch dependencies.",
                manifest_path.display(),
                dependency,
                alias,
                path.display()
            ),
            Self::RemoteDependencyMaterializationError {
                manifest_path,
                alias,
                dependency,
                path,
                message,
            } => write!(
                f,
                "Failed to materialize remote dependency `{}` for alias `@{}` declared in {} at {}: {}",
                dependency,
                alias,
                manifest_path.display(),
                path.display(),
                message
            ),
            Self::DependencyAliasCollision { package, alias } => write!(
                f,
                "Package `{package}` has a dependency alias collision for `@{alias}`"
            ),
            Self::DuplicatePackageId {
                package,
                first_root,
                second_root,
            } => write!(
                f,
                "Duplicate package identity `{package}` for {} and {}",
                first_root.display(),
                second_root.display()
            ),
            Self::PackageCycle { cycle } => {
                let cycle = cycle
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" -> ");
                write!(f, "Package dependency cycle detected: {cycle}")
            }
            Self::UnknownBuiltinPackage { name } => {
                write!(f, "Unknown builtin package {name}")
            }
            Self::DirectDependencyOnBuiltin { package } => {
                write!(f, "Cannot directly depend on builtin package {package}")
            }
            Self::BuiltinDependencyOnNonBuiltin { package } => {
                write!(
                    f,
                    "Builtin package cannot depend on non-builtin package {package}"
                )
            }
        }
    }
}

impl WorkspaceDiscoveryError {
    pub fn spans(&self) -> (Span, Vec<Span>) {
        match self {
            Self::Load(error) => error.spans(),
            Self::PackageRootNotFound { .. }
            | Self::ManifestReadError { .. }
            | Self::ManifestParseError { .. }
            | Self::SrcDirectoryMissing { .. }
            | Self::PathCanonicalizationError { .. }
            | Self::MissingDependencyPackage { .. }
            | Self::InvalidDependencyPath { .. }
            | Self::InvalidRemoteDependencySource { .. }
            | Self::MissingRemoteDependencyPackage { .. }
            | Self::RemoteDependencyMaterializationError { .. }
            | Self::DependencyAliasCollision { .. }
            | Self::DuplicatePackageId { .. }
            | Self::PackageCycle { .. }
            | Self::UnknownBuiltinPackage { .. }
            | Self::DirectDependencyOnBuiltin { .. }
            | Self::BuiltinDependencyOnNonBuiltin { .. } => (Span::None, vec![]),
        }
    }

    pub fn to_report(&self) -> miette::Report {
        match self {
            Self::Load(error) => error.to_report(),
            Self::PackageRootNotFound { .. }
            | Self::ManifestReadError { .. }
            | Self::ManifestParseError { .. }
            | Self::SrcDirectoryMissing { .. }
            | Self::PathCanonicalizationError { .. }
            | Self::MissingDependencyPackage { .. }
            | Self::InvalidDependencyPath { .. }
            | Self::InvalidRemoteDependencySource { .. }
            | Self::MissingRemoteDependencyPackage { .. }
            | Self::RemoteDependencyMaterializationError { .. }
            | Self::DependencyAliasCollision { .. }
            | Self::DuplicatePackageId { .. }
            | Self::PackageCycle { .. }
            | Self::UnknownBuiltinPackage { .. }
            | Self::DirectDependencyOnBuiltin { .. }
            | Self::BuiltinDependencyOnNonBuiltin { .. } => message_report(self.to_string()),
        }
    }
}

impl std::error::Error for WorkspaceDiscoveryError {}

#[derive(Debug, Clone)]
pub enum WorkspaceError {
    LowerError {
        file: FileName,
        source: Arc<str>,
        error: CompileError,
    },
    UnknownDependency {
        source: Arc<str>,
        span: Span,
        dependency: String,
    },
    ImportedModuleNotFound {
        source: Arc<str>,
        span: Span,
        import_path: String,
    },
    DuplicateImportAlias {
        source: Arc<str>,
        span: Span,
        alias: String,
    },
    BindingNameConflictsWithImportAlias {
        source: Arc<str>,
        span: Span,
        name: String,
    },
    UnknownModuleQualifier {
        source: Arc<str>,
        span: Span,
        qualifier: String,
        name: String,
    },
    QualifiedCurrentModuleReference {
        source: Arc<str>,
        span: Span,
        qualifier: String,
        name: String,
    },
    UnattachedExternalModule {
        package: PackageId,
        module_path: String,
    },
}

impl Display for WorkspaceError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::LowerError { .. }
            | Self::UnknownDependency { .. }
            | Self::ImportedModuleNotFound { .. }
            | Self::DuplicateImportAlias { .. }
            | Self::BindingNameConflictsWithImportAlias { .. }
            | Self::UnknownModuleQualifier { .. }
            | Self::QualifiedCurrentModuleReference { .. } => write!(f, "{:?}", self.to_report()),
            Self::UnattachedExternalModule { .. } => write!(f, "{}", self.plain_message()),
        }
    }
}

impl WorkspaceError {
    fn plain_message(&self) -> String {
        match self {
            Self::LowerError { file, .. } => format!("Failed to lower source file {}", file.0),
            Self::UnknownDependency { dependency, .. } => {
                format!("Unknown dependency alias `@{dependency}`")
            }
            Self::ImportedModuleNotFound { import_path, .. } => {
                format!("Imported module `{}` was not found", import_path)
            }
            Self::DuplicateImportAlias { alias, .. } => {
                format!("Duplicate import alias `{}`", alias)
            }
            Self::BindingNameConflictsWithImportAlias { name, .. } => {
                format!(
                    "Top-level binding `{}` conflicts with an import alias",
                    name
                )
            }
            Self::UnknownModuleQualifier {
                qualifier, name, ..
            } => format!(
                "Unknown module qualifier `{}` in reference `{}`",
                qualifier, name
            ),
            Self::QualifiedCurrentModuleReference {
                qualifier, name, ..
            } => format!(
                "Reference `{}` uses qualifier `{}` for current module; use unqualified name",
                name, qualifier
            ),
            Self::UnattachedExternalModule {
                package,
                module_path,
            } => format!(
                "External module `{}` was not attached to any parsed module in package `{}`",
                module_path, package
            ),
        }
    }

    pub fn spans(&self) -> (Span, Vec<Span>) {
        match self {
            Self::LowerError { error, .. } => (error.span(), vec![]),
            Self::UnknownDependency { span, .. }
            | Self::ImportedModuleNotFound { span, .. }
            | Self::DuplicateImportAlias { span, .. }
            | Self::BindingNameConflictsWithImportAlias { span, .. }
            | Self::UnknownModuleQualifier { span, .. }
            | Self::QualifiedCurrentModuleReference { span, .. } => (span.clone(), vec![]),
            Self::UnattachedExternalModule { .. } => (Span::None, vec![]),
        }
    }

    pub fn to_report(&self) -> miette::Report {
        match self {
            Self::LowerError { source, error, .. } => error.to_report(source.clone()),
            Self::UnknownDependency { source, span, .. }
            | Self::ImportedModuleNotFound { source, span, .. }
            | Self::DuplicateImportAlias { source, span, .. }
            | Self::BindingNameConflictsWithImportAlias { source, span, .. }
            | Self::UnknownModuleQualifier { source, span, .. }
            | Self::QualifiedCurrentModuleReference { source, span, .. } => {
                report_with_source_span(source.clone(), span, self.plain_message())
            }
            Self::UnattachedExternalModule { .. } => message_report(self.plain_message()),
        }
    }
}

impl std::error::Error for WorkspaceError {}

#[derive(Debug, Clone)]
pub struct WorkspacePackage {
    pub id: PackageId,
    pub parsed: ParsedPackage,
    pub dependencies: BTreeMap<String, PackageId>,
    pub externals: BTreeMap<ModulePath, ExternalModule>,
}

impl WorkspacePackage {
    pub fn new(id: PackageId, parsed: ParsedPackage) -> Self {
        Self {
            id,
            parsed,
            dependencies: BTreeMap::new(),
            externals: BTreeMap::new(),
        }
    }

    pub fn with_dependency(mut self, alias: impl Into<String>, package: PackageId) -> Self {
        self.dependencies.insert(alias.into(), package);
        self
    }

    pub fn with_dependencies(
        mut self,
        dependencies: impl IntoIterator<Item = (impl Into<String>, PackageId)>,
    ) -> Self {
        for (alias, package) in dependencies {
            self.dependencies.insert(alias.into(), package);
        }
        self
    }

    pub fn with_external(mut self, module_path: ModulePath, module: ExternalModule) -> Self {
        self.externals.insert(module_path, module);
        self
    }

    pub fn with_externals(
        mut self,
        externals: impl IntoIterator<Item = (ModulePath, ExternalModule)>,
    ) -> Self {
        for (module_path, module) in externals {
            self.externals.insert(module_path, module);
        }
        self
    }
}

#[derive(Debug, Clone)]
pub struct WorkspacePackages {
    pub root_package: PackageId,
    pub packages: Vec<WorkspacePackage>,
}

#[derive(Debug, Clone)]
pub struct DiscoveredPackage {
    pub id: PackageId,
    pub layout: PackageLayout,
    pub manifest: PackageManifest,
    pub dependencies: BTreeMap<String, PackageId>,
}

#[derive(Debug, Clone)]
pub struct PackageGraph {
    pub root_package: PackageId,
    pub packages: Vec<DiscoveredPackage>,
}

#[derive(Debug, Clone)]
pub struct FileImportScope<S> {
    pub current_module: S,
    pub aliases: BTreeMap<String, S>,
    pub package_aliases: BTreeMap<PackageId, String>,
}

#[derive(Debug, Clone)]
pub struct Workspace {
    root_package: PackageId,
    lowered: Module<Arc<process::Expression<(), Universal>>, Universal>,
    docs: Docs<Universal>,
    visibility: VisibilityIndex,
    package_modules: BTreeMap<PackageId, Vec<ModulePath>>,
    file_scopes: HashMap<FileName, FileImportScope<Universal>>,
    import_spans: HashMap<FileName, Vec<(Span, Universal)>>,
    sources: HashMap<FileName, Arc<str>>,
}

impl Workspace {
    pub fn lowered_module(&self) -> &Module<Arc<process::Expression<(), Universal>>, Universal> {
        &self.lowered
    }

    pub fn docs(&self) -> &Docs<Universal> {
        &self.docs
    }

    pub fn packages(&self) -> Vec<PackageId> {
        self.package_modules.keys().cloned().collect()
    }

    pub fn root_package(&self) -> &PackageId {
        &self.root_package
    }

    pub fn root_modules(&self) -> Vec<ModulePath> {
        self.modules_in_package(&self.root_package).to_vec()
    }

    pub fn modules_in_package(&self, package: &PackageId) -> &[ModulePath] {
        self.package_modules
            .get(package)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn import_scope(&self, file: &FileName) -> Option<&FileImportScope<Universal>> {
        self.file_scopes.get(file)
    }

    pub fn import_spans(&self) -> &HashMap<FileName, Vec<(Span, Universal)>> {
        &self.import_spans
    }

    pub fn sources(&self) -> &HashMap<FileName, Arc<str>> {
        &self.sources
    }

    pub fn type_doc(&self, name: &GlobalName<Universal>) -> Option<&DocComment> {
        self.docs.type_doc(name)
    }

    pub fn module_doc(&self, module: &Universal) -> Option<&DocComment> {
        self.docs.module_doc(module)
    }

    pub fn declaration_doc(&self, name: &GlobalName<Universal>) -> Option<&DocComment> {
        self.docs.declaration_doc(name)
    }

    pub fn declaration_visibility(&self, name: &GlobalName<Universal>) -> Visibility {
        self.visibility.declaration_visibility(name)
    }

    pub fn type_check(&self) -> (CheckedWorkspace, Vec<TypeError<Universal>>) {
        let mut errors = IndexSet::new();
        errors.extend(validate_visibility(
            &self.lowered,
            &self.visibility,
            &self.file_scopes,
            &self.import_spans,
        ));

        let (checked, type_errors) = self.lowered.type_check();
        errors.extend(type_errors);
        let hover_index = HoverIndex::new(
            &checked,
            &self.docs,
            &self.import_spans,
            |file, _module, name| {
                let Some(scope) = self.file_scopes.get(file) else {
                    return true;
                };
                self.visibility
                    .type_visible_from(&scope.current_module, name)
            },
            |file, _module, name| {
                let Some(scope) = self.file_scopes.get(file) else {
                    return true;
                };
                self.visibility
                    .declaration_visible_from(&scope.current_module, name)
            },
        );
        (
            CheckedWorkspace {
                workspace: self.clone(),
                checked,
                hover_index,
            },
            errors.into_iter().collect(),
        )
    }
}

#[derive(Clone)]
pub struct CheckedWorkspace {
    workspace: Workspace,
    checked: CheckedModule<Universal>,
    hover_index: HoverIndex<Universal>,
}

impl CheckedWorkspace {
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub fn checked_module(&self) -> &CheckedModule<Universal> {
        &self.checked
    }

    pub fn hover_index(&self) -> &HoverIndex<Universal> {
        &self.hover_index
    }

    pub fn hover_at(
        &self,
        file: &FileName,
        row: u32,
        column: u32,
    ) -> Option<process::HoverInfo<Universal>> {
        self.hover_index.query(file, row, column)
    }

    pub fn compile_runtime(
        &self,
        max_interactions: u32,
    ) -> Result<Compiled<Unlinked>, RuntimeCompilerError> {
        Compiled::compile_file(&self.checked, max_interactions)
    }

    pub fn render_global_in_file(&self, file: &FileName, name: &GlobalName<Universal>) -> String {
        render_global_name_in_scope(self.workspace.import_scope(file), name)
    }

    pub fn render_type_in_file(
        &self,
        file: &FileName,
        typ: &Type<Universal>,
        indent: usize,
    ) -> String {
        render_type_in_scope(self.workspace.import_scope(file), typ, indent)
    }

    pub fn render_hover_signature_in_file(
        &self,
        file: &FileName,
        hover: &process::HoverInfo<Universal>,
    ) -> String {
        let mut output = String::new();
        if let Some((module, types, declarations)) = hover.module_items() {
            return self.render_module_signature_in_file(file, module, types, declarations);
        }
        if hover.is_type() {
            if let Some(global_name) = hover.global_name() {
                let _ = write!(output, "type ");
                let _ = write_global_name_in_file(
                    &mut output,
                    self.workspace.import_scope(file),
                    global_name,
                );
                if let Some(header) = hover.type_header() {
                    let _ = write_type_hover_header_in_file(
                        &mut output,
                        self.workspace.import_scope(file),
                        header,
                    );
                }
                let _ = write!(output, " = ");
            }
        } else if hover.is_declaration() {
            if let Some(global_name) = hover.global_name() {
                let _ = write!(output, "dec ");
                let _ = write_global_name_in_file(
                    &mut output,
                    self.workspace.import_scope(file),
                    global_name,
                );
                let _ = write!(output, " : ");
            }
        } else if let Some(name) = hover.variable_name() {
            let _ = write!(output, "{} : ", name);
        }
        if let Some(typ) = hover.typ() {
            let _ = write_type_in_scope_with_options(
                &mut output,
                self.workspace.import_scope(file),
                typ,
                TypeRenderOptions::pretty(0)
                    .with_prefer_display_hints(hover.prefer_display_hints()),
            );
        }
        output
    }

    fn render_module_signature_in_file(
        &self,
        file: &FileName,
        module: &Universal,
        types: &[(GlobalName<Universal>, Vec<TypeParameter>, Type<Universal>)],
        declarations: &[(GlobalName<Universal>, Type<Universal>)],
    ) -> String {
        let scope = self.workspace.import_scope(file);
        let mut output = String::new();
        let _ = write!(output, "module ");
        let _ = write_universal_module_in_file(&mut output, scope, module);
        for (name, params, _typ) in types {
            output.push('\n');
            let _ = write!(output, "type ");
            let _ = write_global_name_in_file(&mut output, scope, name);
            if !params.is_empty() {
                let _ = write!(
                    output,
                    "<{}>",
                    params
                        .iter()
                        .map(|p| p.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
        for (name, _typ) in declarations {
            output.push('\n');
            let _ = write!(output, "dec ");
            let _ = write_global_name_in_file(&mut output, scope, name);
        }
        output
    }

    pub fn render_hover_markdown_in_file(
        &self,
        file: &FileName,
        hover: &process::HoverInfo<Universal>,
    ) -> String {
        let signature = self.render_hover_signature_in_file(file, hover);
        if let Some(doc) = hover.doc() {
            format!("```par\n{signature}\n```\n\n{}", doc.markdown)
        } else {
            format!("```par\n{signature}\n```")
        }
    }
}

pub fn render_global_name_in_scope(
    scope: Option<&FileImportScope<Universal>>,
    name: &GlobalName<Universal>,
) -> String {
    let mut output = String::new();
    let _ = write_global_name_in_file(&mut output, scope, name);
    output
}

pub fn render_type_in_scope(
    scope: Option<&FileImportScope<Universal>>,
    typ: &Type<Universal>,
    indent: usize,
) -> String {
    let mut output = String::new();
    let _ = write_type_in_scope_with_options(
        &mut output,
        scope,
        typ,
        TypeRenderOptions::pretty(indent),
    );
    output
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct AbsoluteModuleLookupKey {
    package: PackageId,
    directories: Vec<String>,
    module_lower: String,
}

pub fn assemble_workspace(
    workspace_packages: WorkspacePackages,
) -> Result<Workspace, WorkspaceError> {
    let WorkspacePackages {
        root_package,
        packages,
    } = workspace_packages;
    let module_lookup = build_module_lookup(&packages);
    let package_modules = package_module_paths(&packages);

    let mut lowered = Module::default();
    let mut module_docs = IndexMap::new();
    let mut visibility = VisibilityIndex::default();
    let mut sources = HashMap::<FileName, Arc<str>>::new();
    let mut file_scopes = HashMap::<FileName, FileImportScope<Universal>>::new();
    let mut import_spans = HashMap::<FileName, Vec<(Span, Universal)>>::new();

    for package in packages {
        load_package(
            &mut lowered,
            &mut module_docs,
            &mut visibility,
            &mut sources,
            &mut file_scopes,
            &mut import_spans,
            &module_lookup,
            package,
        )?;
    }

    let mut docs = lowered.docs();
    docs.modules = module_docs;
    Ok(Workspace {
        root_package,
        lowered,
        docs,
        visibility,
        package_modules,
        file_scopes,
        import_spans,
        sources,
    })
}

fn collect_source_files(
    layout: &PackageLayout,
    overrides: Option<&SourceOverrides>,
) -> Result<Vec<LoadedPackageFile>, PackageLoadError> {
    let mut files = Vec::new();
    collect_source_files_recursive(&layout.src_dir, &layout.src_dir, &mut files)?;
    if let Some(overrides) = overrides {
        let normalized_overrides = overrides
            .iter()
            .map(|(path, source)| (normalized_path_key(path), source))
            .collect::<HashMap<_, _>>();
        for file in &mut files {
            let file_path = Path::new(file.name.0.as_str());
            let Some(source) = normalized_overrides.get(&normalized_path_key(file_path)) else {
                continue;
            };
            file.source = (*source).clone();
        }
    }
    files.sort_by(|a, b| a.relative_path_from_src.cmp(&b.relative_path_from_src));
    Ok(files)
}

pub fn load_package_source_files(
    layout: &PackageLayout,
) -> Result<Vec<LoadedPackageFile>, PackageLoadError> {
    collect_source_files(layout, None)
}

pub fn parse_loaded_files(
    files: Vec<LoadedPackageFile>,
) -> Result<ParsedPackage, PackageLoadError> {
    let mut modules_by_key: BTreeMap<ModulePathKey, ParsedModule> = BTreeMap::new();

    for file in files {
        let (file_module_name, module_part_suffix) =
            parse_module_name_from_file_name(&file.relative_path_from_src)?;
        let source: Arc<str> = Arc::from(file.source.as_str());
        let source_file = parse_source_file(&file.source, file.name.clone()).map_err(|error| {
            PackageLoadError::ParseError {
                file: file.name.clone(),
                source: Arc::clone(&source),
                error,
            }
        })?;

        let declared_module_name = source_file
            .module_decl
            .as_ref()
            .map(|module_decl| module_decl.name.clone())
            .ok_or_else(|| PackageLoadError::MissingModuleDeclaration {
                file: file.name.clone(),
            })?;

        if !declared_module_name.eq_ignore_ascii_case(&file_module_name) {
            return Err(PackageLoadError::FileNameModuleMismatch {
                file: file.name.clone(),
                declared_module: declared_module_name,
                file_module_name,
            });
        }

        let module_path = derive_module_path(&file.relative_path_from_src, &declared_module_name)?;
        let path_key = module_path.key();

        let parsed_file = ParsedPackageFile {
            name: file.name,
            relative_path_from_src: file.relative_path_from_src,
            source,
            module_part_suffix,
            source_file,
        };

        match modules_by_key.entry(path_key) {
            Entry::Vacant(vacant) => {
                vacant.insert(ParsedModule {
                    path: module_path,
                    doc: None,
                    files: vec![parsed_file],
                });
            }
            Entry::Occupied(mut occupied) => {
                let existing_module = occupied.get_mut();
                if !existing_module.path.module.eq(&module_path.module) {
                    let first_file = &existing_module.files[0];
                    return Err(PackageLoadError::ConflictingModuleNameCasing {
                        module_path: existing_module.path.to_slash_path(),
                        first_file: first_file.name.clone(),
                        first_declared_name: existing_module.path.module.clone(),
                        second_file: parsed_file.name.clone(),
                        second_declared_name: module_path.module.clone(),
                    });
                }
                existing_module.files.push(parsed_file);
            }
        }
    }

    let mut modules: Vec<ParsedModule> = modules_by_key.into_values().collect();
    for module in &mut modules {
        module.files.sort_by(|left, right| {
            module_file_ordering_key(left).cmp(&module_file_ordering_key(right))
        });
        module.doc = module
            .files
            .iter()
            .find(|file| file.module_part_suffix.is_none())
            .and_then(|file| file.source_file.module_decl.as_ref())
            .and_then(|module_decl| module_decl.doc.clone());
    }
    modules.sort_by(|left, right| left.path.cmp(&right.path));

    Ok(ParsedPackage { modules })
}

#[derive(Debug, Clone)]
pub struct MissingRemotePackageRequest {
    pub root_layout: PackageLayout,
    pub declaring_layout: PackageLayout,
    pub alias: String,
    pub dependency: String,
    pub destination: PathBuf,
}

pub trait MissingRemotePackageHandler {
    fn handle_missing_remote(
        &mut self,
        request: &MissingRemotePackageRequest,
    ) -> Result<(), WorkspaceDiscoveryError>;
}

struct NoopMissingRemotePackageHandler;

impl MissingRemotePackageHandler for NoopMissingRemotePackageHandler {
    fn handle_missing_remote(
        &mut self,
        _request: &MissingRemotePackageRequest,
    ) -> Result<(), WorkspaceDiscoveryError> {
        Ok(())
    }
}

impl PackageGraph {
    pub fn discover_from_path(start: impl AsRef<Path>) -> Result<Self, WorkspaceDiscoveryError> {
        let mut handler = NoopMissingRemotePackageHandler;
        Self::discover_from_path_with_handler(start, &mut handler)
    }

    pub fn discover_from_path_with_handler(
        start: impl AsRef<Path>,
        remote_handler: &mut impl MissingRemotePackageHandler,
    ) -> Result<Self, WorkspaceDiscoveryError> {
        let layout = PackageLayout::find_from(start)?;
        let root_dir = canonicalize_path(&layout.root_dir)?;
        let mut discovery = PackageGraphDiscovery::new(layout.clone(), remote_handler);
        let root_package = discovery.discover(
            layout,
            root_dir.clone(),
            package_id_from_local_path(&root_dir)?,
        )?;
        Ok(discovery.finish(root_package))
    }

    pub fn into_workspace_packages(
        self,
        overrides: Option<&SourceOverrides>,
    ) -> Result<WorkspacePackages, WorkspaceDiscoveryError> {
        let packages = self
            .packages
            .into_iter()
            .map(|package| {
                let files = collect_source_files(&package.layout, overrides)
                    .map_err(WorkspaceDiscoveryError::Load)?;
                let parsed = parse_loaded_files(files).map_err(WorkspaceDiscoveryError::Load)?;
                Ok(WorkspacePackage::new(package.id, parsed)
                    .with_dependencies(package.dependencies))
            })
            .collect::<Result<Vec<_>, WorkspaceDiscoveryError>>()?;
        Ok(WorkspacePackages {
            root_package: self.root_package,
            packages,
        })
    }
}

pub fn discover_workspace_packages_from_path(
    start: impl AsRef<Path>,
    overrides: Option<&SourceOverrides>,
) -> Result<WorkspacePackages, WorkspaceDiscoveryError> {
    PackageGraph::discover_from_path(start)?.into_workspace_packages(overrides)
}

fn canonicalize_path(path: &Path) -> Result<PathBuf, WorkspaceDiscoveryError> {
    fs::canonicalize(path).map_err(|error| WorkspaceDiscoveryError::PathCanonicalizationError {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn normalized_package_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub fn package_id_from_local_path(path: &Path) -> Result<PackageId, WorkspaceDiscoveryError> {
    let canonical = canonicalize_path(path)?;
    Ok(PackageId::Local(ArcStr::from(normalized_package_path(
        &canonical,
    ))))
}

pub fn package_id_from_remote_source(source: &RemoteDependencySource) -> PackageId {
    PackageId::Remote(ArcStr::from(source.as_str()))
}

fn normalized_path_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

struct PackageGraphDiscovery<'a, H> {
    root_layout: PackageLayout,
    remote_handler: &'a mut H,
    visiting: Vec<PathBuf>,
    package_ids_by_root: BTreeMap<PathBuf, PackageId>,
    roots_by_package_id: BTreeMap<PackageId, PathBuf>,
    packages_by_root: BTreeMap<PathBuf, DiscoveredPackage>,
    root_builtin: bool,
}

impl<'a, H: MissingRemotePackageHandler> PackageGraphDiscovery<'a, H> {
    fn new(root_layout: PackageLayout, remote_handler: &'a mut H) -> Self {
        Self {
            root_layout,
            remote_handler,
            visiting: Vec::new(),
            package_ids_by_root: BTreeMap::new(),
            roots_by_package_id: BTreeMap::new(),
            packages_by_root: BTreeMap::new(),
            root_builtin: false,
        }
    }

    fn finish(self, root_package: PackageId) -> PackageGraph {
        let packages = self.packages_by_root.into_values().collect();
        PackageGraph {
            root_package,
            packages,
        }
    }

    fn discover(
        &mut self,
        layout: PackageLayout,
        canonical_root: PathBuf,
        package_id: PackageId,
    ) -> Result<PackageId, WorkspaceDiscoveryError> {
        if let Some(package_id) = self.package_ids_by_root.get(&canonical_root) {
            return Ok(package_id.clone());
        }

        if let Some(index) = self
            .visiting
            .iter()
            .position(|path| path == &canonical_root)
        {
            let mut cycle = self.visiting[index..].to_vec();
            cycle.push(canonical_root);
            return Err(WorkspaceDiscoveryError::PackageCycle { cycle });
        }

        self.visiting.push(canonical_root.clone());
        let result = self.discover_uncached(layout, canonical_root, package_id);
        self.visiting.pop();
        result
    }

    fn discover_uncached(
        &mut self,
        layout: PackageLayout,
        canonical_root: PathBuf,
        mut package_id: PackageId,
    ) -> Result<PackageId, WorkspaceDiscoveryError> {
        let manifest = PackageManifest::read_from(&layout.manifest_path)?;

        if manifest.builtin {
            let builtin = BuiltinPackage::from_str(&manifest.name).ok_or_else(|| {
                WorkspaceDiscoveryError::UnknownBuiltinPackage {
                    name: manifest.name.clone(),
                }
            })?;
            if self.visiting.len() == 1 {
                // this is the root package
                self.root_builtin = true
            } else if !self.root_builtin {
                return Err(WorkspaceDiscoveryError::DirectDependencyOnBuiltin {
                    package: package_id,
                });
            }
            package_id = PackageId::Builtin(builtin);
        } else if self.root_builtin {
            return Err(WorkspaceDiscoveryError::BuiltinDependencyOnNonBuiltin {
                package: package_id,
            });
        }

        if let Some(first_root) = self.roots_by_package_id.get(&package_id) {
            if first_root != &canonical_root {
                return Err(WorkspaceDiscoveryError::DuplicatePackageId {
                    package: package_id,
                    first_root: first_root.clone(),
                    second_root: canonical_root,
                });
            }
        } else {
            self.roots_by_package_id
                .insert(package_id.clone(), canonical_root.clone());
        }

        let mut dependencies = BTreeMap::new();

        for (alias, dependency) in &manifest.dependencies {
            match dependency {
                DependencySpec::Local(relative_path) => {
                    let dependency_path = layout.resolve_local_dependency(relative_path, alias)?;
                    let dependency_layout = PackageLayout::from_dependency_path(
                        &dependency_path,
                        &layout.manifest_path,
                        alias,
                    )?;
                    let dependency_root = canonicalize_path(&dependency_layout.root_dir)?;
                    let dependency_id = self.discover(
                        dependency_layout,
                        dependency_root.clone(),
                        package_id_from_local_path(&dependency_root)?,
                    )?;
                    dependencies.insert(alias.clone(), dependency_id);
                }
                DependencySpec::Remote(dependency) => {
                    let dependency_path =
                        self.root_layout.managed_remote_dependency_path(dependency);
                    let dependency_layout =
                        match PackageLayout::find_existing_dependency_path(&dependency_path)? {
                            Some(layout) => layout,
                            None => {
                                let request = MissingRemotePackageRequest {
                                    root_layout: self.root_layout.clone(),
                                    declaring_layout: layout.clone(),
                                    alias: alias.clone(),
                                    dependency: dependency.to_string(),
                                    destination: dependency_path.clone(),
                                };
                                self.remote_handler.handle_missing_remote(&request)?;
                                match PackageLayout::find_existing_dependency_path(
                                    &dependency_path,
                                )? {
                                    Some(layout) => layout,
                                    None => {
                                        return Err(
                                        WorkspaceDiscoveryError::MissingRemoteDependencyPackage {
                                            manifest_path: layout.manifest_path.clone(),
                                            alias: alias.clone(),
                                            dependency: dependency.to_string(),
                                            path: dependency_path,
                                        },
                                    );
                                    }
                                }
                            }
                        };
                    let dependency_root = canonicalize_path(&dependency_layout.root_dir)?;
                    let dependency_id = self.discover(
                        dependency_layout,
                        dependency_root,
                        package_id_from_remote_source(dependency),
                    )?;
                    dependencies.insert(alias.clone(), dependency_id);
                }
            }
        }

        let package = DiscoveredPackage {
            id: package_id.clone(),
            layout,
            manifest,
            dependencies,
        };
        self.package_ids_by_root
            .insert(canonical_root.clone(), package_id.clone());
        self.packages_by_root.insert(canonical_root, package);
        Ok(package_id)
    }
}

fn build_module_lookup(
    packages: &[WorkspacePackage],
) -> BTreeMap<AbsoluteModuleLookupKey, ModulePath> {
    let mut lookup = BTreeMap::new();
    for package in packages {
        for module in &package.parsed.modules {
            lookup.insert(
                module_lookup_key(&package.id, &module.path.directories, &module.path.module),
                module.path.clone(),
            );
        }
    }
    lookup
}

fn package_module_paths(packages: &[WorkspacePackage]) -> BTreeMap<PackageId, Vec<ModulePath>> {
    packages
        .iter()
        .map(|package| {
            (
                package.id.clone(),
                package
                    .parsed
                    .modules
                    .iter()
                    .map(|module| module.path.clone())
                    .collect(),
            )
        })
        .collect()
}

fn load_package(
    lowered: &mut Module<Arc<process::Expression<(), Universal>>, Universal>,
    module_docs: &mut IndexMap<Universal, DocComment>,
    visibility: &mut VisibilityIndex,
    sources: &mut HashMap<FileName, Arc<str>>,
    file_scopes: &mut HashMap<FileName, FileImportScope<Universal>>,
    import_spans: &mut HashMap<FileName, Vec<(Span, Universal)>>,
    module_lookup: &BTreeMap<AbsoluteModuleLookupKey, ModulePath>,
    package: WorkspacePackage,
) -> Result<(), WorkspaceError> {
    let WorkspacePackage {
        id,
        parsed,
        dependencies,
        mut externals,
    } = package;

    for parsed_module in &parsed.modules {
        let current_universal_module = Universal {
            package: id.clone(),
            directories: parsed_module.path.directories.clone(),
            module: parsed_module.path.module.clone(),
        };
        if let Some(doc) = parsed_module.doc.clone() {
            module_docs.insert(current_universal_module.clone(), doc);
        }
        let current_module_path =
            resolved_module_path(&parsed_module.path, ResolvedPackageRef::Local);

        for file in &parsed_module.files {
            sources.insert(file.name.clone(), Arc::clone(&file.source));
            visibility.record_module_export(
                &current_universal_module,
                file.source_file
                    .module_decl
                    .as_ref()
                    .expect("parsed package file missing module declaration")
                    .span
                    .clone(),
                file.source_file
                    .module_decl
                    .as_ref()
                    .expect("parsed package file missing module declaration")
                    .exported,
            );

            let imports = build_file_import_aliases(
                file,
                &current_module_path,
                &id,
                &dependencies,
                module_lookup,
            )?;
            let file_scope = universalize_file_scope(
                &imports,
                &current_module_path,
                &id,
                &dependencies,
                Arc::clone(&file.source),
            )?;
            file_scopes.insert(file.name.clone(), file_scope);

            // Collect import spans for hover info
            for import in &file.source_file.imports {
                let imported_module =
                    resolve_imported_module(import, file, &id, &dependencies, module_lookup)?;
                let universal_module = universalize_module_path(
                    &imported_module,
                    &id,
                    &dependencies,
                    Arc::clone(&file.source),
                )?;
                import_spans
                    .entry(file.name.clone())
                    .or_default()
                    .push((import.span.clone(), universal_module));
            }

            let imported_aliases = imported_aliases(&imports, &current_module_path);
            let mut lowered_file = lower(file.source_file.body.clone()).map_err(|error| {
                WorkspaceError::LowerError {
                    file: file.name.clone(),
                    source: Arc::clone(&file.source),
                    error,
                }
            })?;
            if file.module_part_suffix.is_none() {
                if let Some(external_module) = externals.remove(&parsed_module.path) {
                    merge_module(&mut lowered_file, external_module);
                }
            }
            validate_binding_names(&lowered_file, &imported_aliases, Arc::clone(&file.source))?;
            let resolved_file = resolve_module(
                lowered_file,
                imports,
                &current_module_path,
                Arc::clone(&file.source),
            )?;
            let universal_file = resolve_module_to_universal(
                resolved_file,
                &id,
                &dependencies,
                Arc::clone(&file.source),
            )?;
            visibility.record_module(&universal_file);
            merge_module(lowered, universal_file);
        }
    }

    if let Some((module_path, _)) = externals.into_iter().next() {
        return Err(WorkspaceError::UnattachedExternalModule {
            package: id,
            module_path: module_path.to_slash_path(),
        });
    }

    Ok(())
}

fn build_file_import_aliases(
    file: &ParsedPackageFile,
    current_module_path: &Resolved,
    current_package: &PackageId,
    current_dependencies: &BTreeMap<String, PackageId>,
    module_lookup: &BTreeMap<AbsoluteModuleLookupKey, ModulePath>,
) -> Result<BTreeMap<String, Resolved>, WorkspaceError> {
    let mut aliases = BTreeMap::new();
    let Resolved::Path { module, .. } = current_module_path else {
        unreachable!("current module path should always be a real module path")
    };
    aliases.insert(module.clone(), current_module_path.clone());

    for import in &file.source_file.imports {
        let imported_module = resolve_imported_module(
            import,
            file,
            current_package,
            current_dependencies,
            module_lookup,
        )?;
        let alias = import
            .alias
            .clone()
            .unwrap_or_else(|| match &imported_module {
                Resolved::Path { module, .. } => module.clone(),
                Resolved::BuiltinOperator(_) => {
                    unreachable!("imports should not resolve to builtin operator modules")
                }
            });

        match aliases.entry(alias.clone()) {
            Entry::Vacant(vacant) => {
                vacant.insert(imported_module);
            }
            Entry::Occupied(_) => {
                return Err(WorkspaceError::DuplicateImportAlias {
                    source: Arc::clone(&file.source),
                    span: import.span.clone(),
                    alias,
                });
            }
        }
    }

    Ok(aliases)
}

fn universalize_file_scope(
    aliases: &BTreeMap<String, Resolved>,
    current_module_path: &Resolved,
    current_package: &PackageId,
    dependencies: &BTreeMap<String, PackageId>,
    source: Arc<str>,
) -> Result<FileImportScope<Universal>, WorkspaceError> {
    let mut universal_aliases = BTreeMap::new();
    for (alias, module) in aliases {
        universal_aliases.insert(
            alias.clone(),
            universalize_module_path(module, current_package, dependencies, Arc::clone(&source))?,
        );
    }
    let mut package_aliases = BTreeMap::new();
    for (alias, package_id) in dependencies {
        package_aliases
            .entry(package_id.clone())
            .or_insert_with(|| alias.clone());
    }
    Ok(FileImportScope {
        current_module: universalize_module_path(
            current_module_path,
            current_package,
            dependencies,
            source,
        )?,
        aliases: universal_aliases,
        package_aliases,
    })
}

fn universalize_module_path(
    module: &Resolved,
    current_package: &PackageId,
    dependencies: &BTreeMap<String, PackageId>,
    file_source: Arc<str>,
) -> Result<Universal, WorkspaceError> {
    match module {
        Resolved::Path {
            package,
            directories,
            module,
        } => {
            let package = match package {
                ResolvedPackageRef::Local => current_package.clone(),
                ResolvedPackageRef::Dependency(alias) => {
                    let Some(package_id) = dependencies.get(alias).cloned() else {
                        return Err(WorkspaceError::UnknownDependency {
                            source: file_source,
                            span: Span::None,
                            dependency: alias.clone(),
                        });
                    };
                    package_id
                }
            };

            Ok(Universal {
                package,
                directories: directories.clone(),
                module: module.clone(),
            })
        }
        Resolved::BuiltinOperator(BuiltinOperatorModule::Data) => Ok(Universal {
            package: PackageId::Builtin(BuiltinPackage::Core),
            directories: vec![],
            module: String::from("Data"),
        }),
        Resolved::BuiltinOperator(BuiltinOperatorModule::Number) => Ok(Universal {
            package: PackageId::Builtin(BuiltinPackage::Core),
            directories: vec![],
            module: String::from("Number"),
        }),
        Resolved::BuiltinOperator(BuiltinOperatorModule::String) => Ok(Universal {
            package: PackageId::Builtin(BuiltinPackage::Core),
            directories: vec![],
            module: String::from("String"),
        }),
    }
}

fn imported_aliases(
    aliases: &BTreeMap<String, Resolved>,
    current_module_path: &Resolved,
) -> BTreeSet<String> {
    let mut imported = aliases.keys().cloned().collect::<BTreeSet<_>>();
    if let Resolved::Path { module, .. } = current_module_path {
        imported.remove(module);
    }
    imported
}

fn validate_binding_names(
    module: &Module<Arc<process::Expression<(), Unresolved>>, Unresolved>,
    imported_aliases: &BTreeSet<String>,
    source: Arc<str>,
) -> Result<(), WorkspaceError> {
    for type_def in &module.type_defs {
        if imported_aliases.contains(type_def.name.primary.as_str()) {
            return Err(WorkspaceError::BindingNameConflictsWithImportAlias {
                source,
                span: type_def.name.span.clone(),
                name: type_def.name.primary.clone(),
            });
        }
    }
    for declaration in &module.declarations {
        if imported_aliases.contains(declaration.name.primary.as_str()) {
            return Err(WorkspaceError::BindingNameConflictsWithImportAlias {
                source: Arc::clone(&source),
                span: declaration.name.span.clone(),
                name: declaration.name.primary.clone(),
            });
        }
    }
    for definition in &module.definitions {
        if imported_aliases.contains(definition.name.primary.as_str()) {
            return Err(WorkspaceError::BindingNameConflictsWithImportAlias {
                source: Arc::clone(&source),
                span: definition.name.span.clone(),
                name: definition.name.primary.clone(),
            });
        }
    }
    Ok(())
}

fn resolve_imported_module(
    import: &ImportDecl,
    file: &ParsedPackageFile,
    current_package: &PackageId,
    current_dependencies: &BTreeMap<String, PackageId>,
    module_lookup: &BTreeMap<AbsoluteModuleLookupKey, ModulePath>,
) -> Result<Resolved, WorkspaceError> {
    let (resolved_package, absolute_package) = match import.path.dependency.as_deref() {
        None => (ResolvedPackageRef::Local, current_package.clone()),
        Some(alias) => {
            let Some(package_id) = current_dependencies.get(alias).cloned() else {
                return Err(WorkspaceError::UnknownDependency {
                    source: Arc::clone(&file.source),
                    span: import.span.clone(),
                    dependency: alias.to_string(),
                });
            };
            (
                ResolvedPackageRef::Dependency(alias.to_string()),
                package_id,
            )
        }
    };

    let lookup_key = module_lookup_key(
        &absolute_package,
        &import
            .path
            .directories
            .iter()
            .map(|segment| segment.to_lowercase())
            .collect::<Vec<_>>(),
        &import.path.module,
    );

    let canonical =
        module_lookup
            .get(&lookup_key)
            .ok_or_else(|| WorkspaceError::ImportedModuleNotFound {
                source: Arc::clone(&file.source),
                span: import.span.clone(),
                import_path: format_import_path(&import.path),
            })?;

    Ok(Resolved::Path {
        package: resolved_package,
        directories: canonical.directories.clone(),
        module: canonical.module.clone(),
    })
}

fn format_import_path(path: &ImportPath) -> String {
    let mut segments = Vec::new();
    if let Some(dependency) = &path.dependency {
        segments.push(format!("@{dependency}"));
    }
    for directory in &path.directories {
        segments.push(directory.clone());
    }
    segments.push(path.module.clone());
    segments.join("/")
}

fn resolve_module(
    module: Module<Arc<process::Expression<(), Unresolved>>, Unresolved>,
    imports: BTreeMap<String, Resolved>,
    current_module_path: &Resolved,
    file_source: Arc<str>,
) -> Result<Module<Arc<process::Expression<(), Resolved>>, Resolved>, WorkspaceError> {
    module.map_global_names(|name| {
        resolve_name_to_resolved(
            name,
            &imports,
            current_module_path,
            Arc::clone(&file_source),
        )
    })
}

fn resolve_module_to_universal(
    module: Module<Arc<process::Expression<(), Resolved>>, Resolved>,
    current_package: &PackageId,
    dependencies: &BTreeMap<String, PackageId>,
    file_source: Arc<str>,
) -> Result<Module<Arc<process::Expression<(), Universal>>, Universal>, WorkspaceError> {
    module.map_global_names(|name| {
        resolve_name_to_universal(
            name,
            current_package,
            dependencies,
            Arc::clone(&file_source),
        )
    })
}

fn resolve_name_to_resolved(
    mut name: GlobalName<Unresolved>,
    imports: &BTreeMap<String, Resolved>,
    current_module_path: &Resolved,
    file_source: Arc<str>,
) -> Result<GlobalName<Resolved>, WorkspaceError> {
    if let Unresolved::BuiltinOperator(module) = name.module {
        return Ok(GlobalName::new(
            name.span,
            Resolved::BuiltinOperator(module),
            name.primary,
        ));
    }

    let Unresolved::Path { qualifier } = &mut name.module else {
        unreachable!("builtin operator case should have returned above")
    };

    if let Some(module_qualifier) = qualifier.take() {
        let Some(target_module) = imports.get(module_qualifier.as_str()) else {
            return Err(WorkspaceError::UnknownModuleQualifier {
                source: file_source,
                span: name.span.clone(),
                qualifier: module_qualifier.clone(),
                name: format!("{}.{}", module_qualifier, name.primary),
            });
        };

        if target_module == current_module_path {
            return Err(WorkspaceError::QualifiedCurrentModuleReference {
                source: file_source,
                span: name.span.clone(),
                qualifier: module_qualifier.clone(),
                name: format!("{}.{}", module_qualifier, name.primary),
            });
        }

        return Ok(GlobalName::new(
            name.span,
            target_module.clone(),
            name.primary,
        ));
    }

    let (module, primary) = if let Some(target_module) = imports.get(&name.primary) {
        let primary = match target_module {
            Resolved::Path { module, .. } => module.clone(),
            Resolved::BuiltinOperator(_) => name.primary.clone(),
        };
        (target_module.clone(), primary)
    } else {
        (current_module_path.clone(), name.primary)
    };
    Ok(GlobalName::new(name.span, module, primary))
}

fn resolve_name_to_universal(
    name: GlobalName<Resolved>,
    current_package: &PackageId,
    dependencies: &BTreeMap<String, PackageId>,
    file_source: Arc<str>,
) -> Result<GlobalName<Universal>, WorkspaceError> {
    let module = match name.module {
        Resolved::Path {
            package,
            directories,
            module,
        } => {
            let package = match package {
                ResolvedPackageRef::Local => current_package.clone(),
                ResolvedPackageRef::Dependency(alias) => {
                    let Some(package_id) = dependencies.get(&alias).cloned() else {
                        return Err(WorkspaceError::UnknownDependency {
                            source: file_source,
                            span: name.span.clone(),
                            dependency: alias.clone(),
                        });
                    };
                    package_id
                }
            };

            Universal {
                package,
                directories,
                module,
            }
        }
        Resolved::BuiltinOperator(BuiltinOperatorModule::Data) => Universal {
            package: PackageId::Builtin(BuiltinPackage::Core),
            directories: vec![],
            module: String::from("Data"),
        },
        Resolved::BuiltinOperator(BuiltinOperatorModule::Number) => Universal {
            package: PackageId::Builtin(BuiltinPackage::Core),
            directories: vec![],
            module: String::from("Number"),
        },
        Resolved::BuiltinOperator(BuiltinOperatorModule::String) => Universal {
            package: PackageId::Builtin(BuiltinPackage::Core),
            directories: vec![],
            module: String::from("String"),
        },
    };

    Ok(GlobalName::new(name.span, module, name.primary))
}

fn module_lookup_key(
    package: &PackageId,
    directories: &[String],
    module: &str,
) -> AbsoluteModuleLookupKey {
    AbsoluteModuleLookupKey {
        package: package.clone(),
        directories: directories.to_vec(),
        module_lower: module.to_lowercase(),
    }
}

fn resolved_module_path(local: &ModulePath, package: ResolvedPackageRef) -> Resolved {
    Resolved::Path {
        package,
        directories: local.directories.clone(),
        module: local.module.clone(),
    }
}

fn merge_module<Expr, S>(target: &mut Module<Expr, S>, mut other: Module<Expr, S>) {
    target.type_defs.append(&mut other.type_defs);
    target.declarations.append(&mut other.declarations);
    target.definitions.append(&mut other.definitions);
}

fn parse_module_name_from_file_name(
    relative_path_from_src: &Path,
) -> Result<(String, Option<String>), PackageLoadError> {
    let file_name = relative_path_from_src
        .file_name()
        .ok_or_else(|| PackageLoadError::InvalidSourceFileName {
            path: relative_path_from_src.to_path_buf(),
        })?
        .to_str()
        .ok_or_else(|| PackageLoadError::InvalidSourceFilePath {
            path: relative_path_from_src.to_path_buf(),
        })?;

    let module_file_stem =
        file_name
            .strip_suffix(".par")
            .ok_or_else(|| PackageLoadError::InvalidSourceFileName {
                path: relative_path_from_src.to_path_buf(),
            })?;

    let mut segments = module_file_stem.split('.');
    let module_name = segments
        .next()
        .ok_or_else(|| PackageLoadError::InvalidSourceFileName {
            path: relative_path_from_src.to_path_buf(),
        })?;

    if module_name.is_empty() {
        return Err(PackageLoadError::InvalidSourceFileName {
            path: relative_path_from_src.to_path_buf(),
        });
    }

    let mut suffix_segments = Vec::new();
    for segment in segments {
        if segment.is_empty() {
            return Err(PackageLoadError::InvalidSourceFileName {
                path: relative_path_from_src.to_path_buf(),
            });
        }
        suffix_segments.push(segment.to_string());
    }

    let suffix = if suffix_segments.is_empty() {
        None
    } else {
        Some(suffix_segments.join("."))
    };

    Ok((module_name.to_string(), suffix))
}

fn derive_module_path(
    relative_path_from_src: &Path,
    declared_module_name: &str,
) -> Result<ModulePath, PackageLoadError> {
    let mut directories = Vec::new();
    if let Some(parent) = relative_path_from_src.parent() {
        for component in parent.components() {
            let component = component.as_os_str().to_str().ok_or_else(|| {
                PackageLoadError::InvalidSourceFilePath {
                    path: relative_path_from_src.to_path_buf(),
                }
            })?;
            directories.push(component.to_lowercase());
        }
    }

    Ok(ModulePath {
        directories,
        module: declared_module_name.to_string(),
    })
}

fn module_file_ordering_key(file: &ParsedPackageFile) -> (u8, String, PathBuf) {
    match &file.module_part_suffix {
        None => (0, String::new(), file.relative_path_from_src.clone()),
        Some(suffix) => (1, suffix.clone(), file.relative_path_from_src.clone()),
    }
}

fn collect_source_files_recursive(
    src_root: &Path,
    current_dir: &Path,
    files: &mut Vec<LoadedPackageFile>,
) -> Result<(), PackageLoadError> {
    let entries =
        fs::read_dir(current_dir).map_err(|error| PackageLoadError::DirectoryReadError {
            path: current_dir.to_path_buf(),
            message: error.to_string(),
        })?;

    for entry in entries {
        let entry = entry.map_err(|error| PackageLoadError::DirectoryReadError {
            path: current_dir.to_path_buf(),
            message: error.to_string(),
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_source_files_recursive(src_root, &path, files)?;
            continue;
        }

        if path.extension().and_then(|ext| ext.to_str()) != Some("par") {
            continue;
        }

        let source =
            fs::read_to_string(&path).map_err(|error| PackageLoadError::FileReadError {
                path: path.clone(),
                message: error.to_string(),
            })?;

        let relative_path_from_src = path
            .strip_prefix(src_root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| path.clone());

        files.push(LoadedPackageFile {
            name: FileName::from(path.as_path()),
            relative_path_from_src,
            source,
        });
    }
    Ok(())
}

fn write_global_name_in_file(
    f: &mut impl Write,
    scope: Option<&FileImportScope<Universal>>,
    name: &GlobalName<Universal>,
) -> fmt::Result {
    if let Some(scope) = scope {
        if name.module == scope.current_module {
            return write!(f, "{}", name.primary);
        }

        for (alias, module) in &scope.aliases {
            if module == &name.module && name.is_primary_export() {
                return write!(f, "{alias}");
            }
        }
    }

    write_universal_module_in_file(f, scope, &name.module)?;
    write!(f, ".{}", name.primary)
}

fn write_universal_module_in_file(
    f: &mut impl Write,
    scope: Option<&FileImportScope<Universal>>,
    module: &Universal,
) -> fmt::Result {
    if let Some(scope) = scope {
        if module == &scope.current_module {
            return write!(f, "{}", module.module);
        }

        for (alias, imported_module) in &scope.aliases {
            if imported_module == module {
                return write!(f, "{alias}");
            }
        }

        if module.package == scope.current_module.package {
            return write_local_module_path(f, module);
        }

        if let Some(package_alias) = scope.package_aliases.get(&module.package) {
            return write_package_relative_module_path(f, package_alias, module);
        }
    }

    write!(f, "{module}")
}

fn write_local_module_path(f: &mut impl Write, module: &Universal) -> fmt::Result {
    if module.directories.is_empty() {
        write!(f, "{}", module.module)
    } else {
        write!(f, "{}/{}", module.directories.join("/"), module.module)
    }
}

fn write_package_relative_module_path(
    f: &mut impl Write,
    package_alias: &str,
    module: &Universal,
) -> fmt::Result {
    write!(f, "@{package_alias}/")?;
    write_local_module_path(f, module)
}

struct ScopedGlobalNameWriter<'a> {
    scope: Option<&'a FileImportScope<Universal>>,
}

impl GlobalNameWriter<Universal> for ScopedGlobalNameWriter<'_> {
    fn write_global_name<W: Write>(&self, f: &mut W, name: &GlobalName<Universal>) -> fmt::Result {
        write_global_name_in_file(f, self.scope, name)
    }
}

fn write_type_in_scope_with_options(
    f: &mut impl Write,
    scope: Option<&FileImportScope<Universal>>,
    typ: &Type<Universal>,
    options: TypeRenderOptions,
) -> fmt::Result {
    let names = ScopedGlobalNameWriter { scope };
    typ.pretty_with_options(f, &names, options)
}

fn write_type_args_in_file(
    f: &mut impl Write,
    scope: Option<&FileImportScope<Universal>>,
    args: &[Type<Universal>],
) -> fmt::Result {
    if args.is_empty() {
        return Ok(());
    }

    let names = ScopedGlobalNameWriter { scope };
    write!(f, "<")?;
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        arg.pretty_compact(f, &names)?;
    }
    write!(f, ">")
}

fn write_type_hover_header_in_file(
    f: &mut impl Write,
    scope: Option<&FileImportScope<Universal>>,
    header: &process::TypeHoverHeader<Universal>,
) -> fmt::Result {
    match header {
        process::TypeHoverHeader::Parameters(params) => write_type_parameters(f, params),
        process::TypeHoverHeader::Arguments(args) => write_type_args_in_file(f, scope, args),
    }
}

fn write_type_parameters(f: &mut impl Write, params: &[TypeParameter]) -> fmt::Result {
    if params.is_empty() {
        return Ok(());
    }
    write!(f, "<")?;
    for (i, param) in params.iter().enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{param}")?;
    }
    write!(f, ">")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend_impl::language::TypeConstraint;
    use crate::frontend_impl::types::Visibility;
    use arcstr::literal;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_package_id() -> PackageId {
        PackageId::Special(literal!("__test__"))
    }

    fn parsed_package_from_files(root: &str, files: &[(&str, &str)]) -> ParsedPackage {
        parse_loaded_files(
            files
                .iter()
                .map(|(path, source)| LoadedPackageFile {
                    name: FileName::from(format!("{root}/{path}")),
                    relative_path_from_src: PathBuf::from(path),
                    source: (*source).to_owned(),
                })
                .collect(),
        )
        .unwrap()
    }

    fn workspace_type_errors(packages: Vec<WorkspacePackage>) -> Vec<TypeError<Universal>> {
        let root_package = packages
            .first()
            .map(|package| package.id.clone())
            .unwrap_or_else(test_package_id);
        let (_checked, type_errors) = assemble_workspace(WorkspacePackages {
            root_package,
            packages,
        })
        .unwrap()
        .type_check();
        type_errors
    }

    fn checked_workspace_from_files(root: &str, files: &[(&str, &str)]) -> CheckedWorkspace {
        let parsed = parsed_package_from_files(root, files);
        let (checked, type_errors) = assemble_workspace(WorkspacePackages {
            root_package: test_package_id(),
            packages: vec![WorkspacePackage::new(test_package_id(), parsed)],
        })
        .unwrap()
        .type_check();
        assert!(type_errors.is_empty(), "type errors: {:?}", type_errors);
        checked
    }

    fn checked_workspace_from_source(source: &str) -> CheckedWorkspace {
        checked_workspace_from_files("local", &[("Main.par", source)])
    }

    fn row_and_column(source: &str, index: usize) -> (u32, u32) {
        let (mut row, mut column) = (0, 0);
        for ch in source[..index].chars() {
            if ch == '\n' {
                row += 1;
                column = 0;
            } else {
                column += 1;
            }
        }
        (row, column)
    }

    fn temp_package_root(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("par-core-{prefix}-{unique}"));
        fs::create_dir_all(&root).expect("failed to create temp package root");
        root
    }

    fn write_package(root: &Path, manifest: &str, files: &[(&str, &str)]) {
        fs::create_dir_all(root.join("src")).expect("failed to create src directory");
        fs::write(root.join("Par.toml"), manifest).expect("failed to write manifest");
        for (relative_path, source) in files {
            let path = root.join(relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("failed to create parent directory");
            }
            fs::write(path, source).expect("failed to write source file");
        }
    }

    #[test]
    fn doc_comments_survive_into_checked_workspace_docs() {
        let source = "\
/*Module docs*/
module Main

/*Type docs*/
type Item = !

// Run docs
dec Run : !
def Run = external
";
        let checked = checked_workspace_from_source(source);
        let module = Universal {
            package: test_package_id(),
            directories: Vec::new(),
            module: String::from("Main"),
        };

        let type_name = checked
            .checked_module()
            .type_defs
            .globals
            .keys()
            .find(|name| name.primary == "Item")
            .unwrap();
        let declaration_name = checked
            .checked_module()
            .declarations
            .keys()
            .find(|name| name.primary == "Run")
            .unwrap();

        assert_eq!(
            checked
                .workspace()
                .type_doc(type_name)
                .map(|doc| doc.markdown.as_str()),
            Some("Type docs")
        );
        assert_eq!(
            checked
                .workspace()
                .module_doc(&module)
                .map(|doc| doc.markdown.as_str()),
            Some("Module docs")
        );
        assert_eq!(
            checked
                .workspace()
                .declaration_doc(declaration_name)
                .map(|doc| doc.markdown.as_str()),
            Some("Run docs")
        );
    }

    #[test]
    fn module_doc_comments_only_come_from_main_module_file() {
        let parsed = parsed_package_from_files(
            "local",
            &[
                (
                    "Main.par",
                    "\
/*Main docs*/
module Main
",
                ),
                (
                    "Main.Part.par",
                    "\
/*Part docs*/
module Main
",
                ),
            ],
        );

        assert_eq!(parsed.modules.len(), 1);
        assert_eq!(
            parsed.modules[0]
                .doc
                .as_ref()
                .map(|doc| doc.markdown.as_str()),
            Some("Main docs")
        );
    }

    #[test]
    fn hover_info_includes_doc_comments_for_global_identifiers() {
        let source = "\
module Main

/*Type docs*/
type Item = !

// Run docs
dec Run : Item
def Run = external

dec Main : Item
def Main = Run
";
        let checked = checked_workspace_from_source(source);
        let file = checked.workspace().sources().keys().next().unwrap();

        let item_index = source.match_indices("Item").nth(1).unwrap().0;
        let (item_row, item_column) = row_and_column(source, item_index);
        let item_hover = checked.hover_at(file, item_row, item_column).unwrap();
        assert_eq!(
            item_hover.doc().map(|doc| doc.markdown.as_str()),
            Some("Type docs")
        );
        assert_eq!(
            item_hover.global_name().map(|name| name.primary.as_str()),
            Some("Item")
        );

        let run_index = source.match_indices("Run").last().unwrap().0;
        let (run_row, run_column) = row_and_column(source, run_index);
        let run_hover = checked.hover_at(file, run_row, run_column).unwrap();
        assert_eq!(
            run_hover.doc().map(|doc| doc.markdown.as_str()),
            Some("Run docs")
        );
        assert_eq!(
            run_hover.global_name().map(|name| name.primary.as_str()),
            Some("Run")
        );
    }

    #[test]
    fn import_hover_includes_module_doc_comments() {
        let main_source = "\
module Main
import Helper
";
        let checked = checked_workspace_from_files(
            "local",
            &[
                (
                    "Helper.par",
                    "\
/*Helper docs*/
export module Helper
",
                ),
                ("Main.par", main_source),
            ],
        );
        let file = FileName::from("local/Main.par");

        let helper_index = main_source.match_indices("Helper").last().unwrap().0;
        let (row, column) = row_and_column(main_source, helper_index);
        let hover = checked.hover_at(&file, row, column).unwrap();

        assert!(hover.is_module());
        assert_eq!(
            hover.doc().map(|doc| doc.markdown.as_str()),
            Some("Helper docs")
        );
        assert_eq!(
            checked.render_hover_markdown_in_file(&file, &hover),
            "```par\nmodule Helper\n```\n\nHelper docs"
        );
    }

    #[test]
    fn hover_info_includes_type_for_empty_local_command_chain_before_continuation() {
        let source = "\
module Main

def Main : ! = chan exit {
  let x = !
  x
  exit!
}
";
        let checked = checked_workspace_from_source(source);
        let file = checked.workspace().sources().keys().next().unwrap();

        let x_index = source.match_indices("\n  x\n").next().unwrap().0 + 3;
        let (row, column) = row_and_column(source, x_index);
        let hover = checked.hover_at(file, row, column).unwrap();

        assert_eq!(hover.variable_name(), Some("x"));
        assert_eq!(
            checked.render_hover_signature_in_file(file, &hover),
            "x : !"
        );
    }

    #[test]
    fn constrained_type_parameters_allow_valid_type_arguments() {
        checked_workspace_from_source(
            "\
module Main

dec UseShare : [type a: share] !
def UseShare = [type a: share] !
def Ok = UseShare(type !)
",
        );
    }

    #[test]
    fn constrained_type_parameters_reject_invalid_type_arguments() {
        let source = "\
module Main

dec UseShare : [type a: share] !
def UseShare = [type a: share] !
def Bad = UseShare(type [!] !)
";
        let errors = workspace_type_errors(vec![WorkspacePackage::new(
            test_package_id(),
            parsed_package_from_files("local", &[("Main.par", source)]),
        )]);

        assert!(errors.iter().any(|error| matches!(
            error,
            TypeError::TypeDoesNotSatisfyConstraint(_, name, _, TypeConstraint::Share)
                if name.string.as_str() == "a"
        )));
    }

    #[test]
    fn template_string_interpolation_requires_string() {
        let source = "\
module Main

def Bad = `${1}`
";
        let errors = workspace_type_errors(vec![WorkspacePackage::new(
            test_package_id(),
            parsed_package_from_files("local", &[("Main.par", source)]),
        )]);

        assert!(
            !errors.is_empty(),
            "non-string template interpolation should produce a type error"
        );
    }

    #[test]
    fn template_data_interpolation_requires_data() {
        let source = "\
module Main

def Bad = `#{[x: !] x}`
";
        let errors = workspace_type_errors(vec![WorkspacePackage::new(
            test_package_id(),
            parsed_package_from_files("local", &[("Main.par", source)]),
        )]);

        assert!(
            !errors.is_empty(),
            "non-data template interpolation should produce a type error"
        );
    }

    #[test]
    fn checked_binder_constraints_must_match_exactly() {
        let source = "\
module Main

def ExpectShare : [type a: share, a] a = [type a, x: a] x
";
        let errors = workspace_type_errors(vec![WorkspacePackage::new(
            test_package_id(),
            parsed_package_from_files("local", &[("Main.par", source)]),
        )]);

        assert!(errors.iter().any(|error| matches!(
            error,
            TypeError::TypeParameterConstraintMismatch(_, name, TypeConstraint::Any, TypeConstraint::Share)
                if name.string.as_str() == "a"
        )));
    }

    #[test]
    fn inferred_types_preserve_parameter_constraints_in_rendering() {
        let source = "\
module Main

def Identity = [type a: number, x: a] x
";
        let checked = checked_workspace_from_source(source);
        let file = checked.workspace().sources().keys().next().unwrap();
        let (_name, (_definition, typ)) = checked
            .checked_module()
            .definitions
            .iter()
            .find(|(name, _)| name.primary == "Identity")
            .unwrap();

        assert_eq!(
            checked.render_type_in_file(file, typ, 0),
            "[type a: number, a] a"
        );
    }

    #[test]
    fn signed_inference_promotes_nat_lower_bounds_to_int() {
        let source = "\
module Main

def SignedId : [<a: signed> a] a = [<a: signed> x] x
def Result = SignedId(5)
";
        let checked = checked_workspace_from_source(source);
        let file = checked.workspace().sources().keys().next().unwrap();
        let (_name, (_definition, typ)) = checked
            .checked_module()
            .definitions
            .iter()
            .find(|(name, _)| name.primary == "Result")
            .unwrap();

        assert_eq!(checked.render_type_in_file(file, typ, 0), "Int");
    }

    #[test]
    fn separate_implicit_items_infer_from_their_own_arguments() {
        let source = "\
module Main

def ChooseSecond : [<a: share> a, <b> b] b = [<a: share> first, <b> second] second
def Result = ChooseSecond(5, \"chosen\")
";
        let checked = checked_workspace_from_source(source);
        let file = checked.workspace().sources().keys().next().unwrap();
        let (_name, (_definition, typ)) = checked
            .checked_module()
            .definitions
            .iter()
            .find(|(name, _)| name.primary == "Result")
            .unwrap();

        assert_eq!(checked.render_type_in_file(file, typ, 0), "String");
    }

    #[test]
    fn implicit_existential_item_infers_and_exposes_its_hidden_type() {
        let source = "\
module Main

type Packed = (<a: share> a)!

def PackedUnit : Packed = (!)!
def Unpack : [Packed] ! = [packed]
  let (<a: share> value)! = packed
  in !
def Result = Unpack(PackedUnit)
";
        let checked = checked_workspace_from_source(source);
        let file = checked.workspace().sources().keys().next().unwrap();
        let (_name, (_definition, typ)) = checked
            .checked_module()
            .definitions
            .iter()
            .find(|(name, _)| name.primary == "Result")
            .unwrap();

        assert_eq!(checked.render_type_in_file(file, typ, 0), "!");
    }

    #[test]
    fn empty_local_command_chain_reports_missing_variable() {
        let source = "\
module Main

def Main : ! = do {
  xyzw
} in !
";
        let errors = workspace_type_errors(vec![WorkspacePackage::new(
            test_package_id(),
            parsed_package_from_files("local", &[("Main.par", source)]),
        )]);

        assert!(errors.iter().any(|error| matches!(
            error,
            TypeError::VariableDoesNotExist(_, name) if name.string.as_str() == "xyzw"
        )));
    }

    #[test]
    fn same_package_can_use_package_visible_export() {
        let errors = workspace_type_errors(vec![WorkspacePackage::new(
            test_package_id(),
            parsed_package_from_files(
                "local",
                &[
                    (
                        "Hidden.par",
                        "\
module Hidden

export {
  type Shared = !
}
",
                    ),
                    (
                        "Main.par",
                        "\
module Main
import Hidden

type Uses = Hidden.Shared
",
                    ),
                ],
            ),
        )]);

        assert!(errors.is_empty(), "unexpected type errors: {:?}", errors);
    }

    #[test]
    fn same_package_cannot_use_module_private_name() {
        let errors = workspace_type_errors(vec![WorkspacePackage::new(
            test_package_id(),
            parsed_package_from_files(
                "local",
                &[
                    (
                        "Hidden.par",
                        "\
module Hidden

type Secret = !
",
                    ),
                    (
                        "Main.par",
                        "\
module Main
import Hidden

type Uses = Hidden.Secret
",
                    ),
                ],
            ),
        )]);

        assert!(errors.iter().any(|error| matches!(
            error,
            TypeError::GlobalNameNotVisible(_, name, Visibility::Module)
                if name.primary == "Secret" && name.module.module == "Hidden"
        )));
    }

    #[test]
    fn cross_package_import_requires_exported_module() {
        let dependency_id = PackageId::Remote(literal!("dep"));
        let dependency = WorkspacePackage::new(
            dependency_id.clone(),
            parsed_package_from_files(
                "dep",
                &[(
                    "Hidden.par",
                    "\
module Hidden
",
                )],
            ),
        );
        let local = WorkspacePackage::new(
            test_package_id(),
            parsed_package_from_files(
                "local",
                &[(
                    "Main.par",
                    "\
module Main
import @dep/Hidden
",
                )],
            ),
        )
        .with_dependency("dep", dependency_id);

        let errors = workspace_type_errors(vec![local, dependency]);

        assert!(errors.iter().any(|error| matches!(
            error,
            TypeError::ImportedModuleNotExported(_, module)
                if module.package == PackageId::Remote(literal!("dep"))
                    && module.module == "Hidden"
        )));
    }

    #[test]
    fn public_export_cannot_expose_package_private_type() {
        let errors = workspace_type_errors(vec![WorkspacePackage::new(
            PackageId::Remote(literal!("dep")),
            parsed_package_from_files(
                "dep",
                &[
                    (
                        "Hidden.par",
                        "\
module Hidden

export {
  type HiddenType = !
}
",
                    ),
                    (
                        "Api.par",
                        "\
export module Api
import Hidden

export {
  type Wrapper = Hidden.HiddenType
}
",
                    ),
                ],
            ),
        )]);

        assert!(errors.iter().any(|error| matches!(
            error,
            TypeError::VisibleItemExposesHiddenType(_, item, Visibility::Public, hidden, Visibility::Package)
                if item.primary == "Wrapper"
                    && item.module.module == "Api"
                    && hidden.primary == "HiddenType"
                    && hidden.module.module == "Hidden"
        )));
    }

    #[test]
    fn multi_file_modules_must_agree_on_export_module() {
        let errors = workspace_type_errors(vec![WorkspacePackage::new(
            test_package_id(),
            parsed_package_from_files(
                "local",
                &[
                    (
                        "Shared.par",
                        "\
export module Shared
",
                    ),
                    (
                        "Shared.Part.par",
                        "\
module Shared
",
                    ),
                ],
            ),
        )]);

        assert!(errors.iter().any(|error| matches!(
            error,
            TypeError::ModuleExportInconsistent(_, _, module)
                if module.package == test_package_id() && module.module == "Shared"
        )));
    }

    #[test]
    fn discovery_uses_canonical_root_path_for_root_package_id_and_root_modules() {
        let root = temp_package_root("root-package-id");
        write_package(
            &root,
            "\
[package]
name = \"root\"
",
            &[(
                "src/Main.par",
                "\
module Main

dec Main : !
def Main = !
",
            )],
        );

        let workspace_packages = discover_workspace_packages_from_path(&root, None).unwrap();
        let canonical_root = canonicalize_path(&root).unwrap();
        assert_eq!(
            workspace_packages.root_package,
            package_id_from_local_path(&canonical_root).unwrap()
        );

        let workspace = assemble_workspace(workspace_packages).unwrap();
        assert_eq!(
            workspace.root_package(),
            &package_id_from_local_path(&canonical_root).unwrap()
        );
        assert_eq!(
            workspace
                .root_modules()
                .into_iter()
                .map(|module| module.to_slash_path())
                .collect::<Vec<_>>(),
            vec![String::from("Main")]
        );
    }

    #[test]
    fn local_path_dependency_import_works() {
        let root = temp_package_root("local-dependency");
        let helper = root.join("helper");
        write_package(
            &helper,
            "\
[package]
name = \"helper\"
",
            &[(
                "src/Util.par",
                "\
export module Util

export {
  dec Run : !
}

def Run = !
",
            )],
        );
        write_package(
            &root,
            "\
[package]
name = \"root\"

[dependencies]
helper = \"./helper\"
",
            &[(
                "src/Main.par",
                "\
module Main
import @helper/Util

dec Main : !
def Main = Util.Run
",
            )],
        );

        let workspace =
            assemble_workspace(discover_workspace_packages_from_path(&root, None).unwrap())
                .unwrap();
        let (_checked, type_errors) = workspace.type_check();
        assert!(type_errors.is_empty(), "type errors: {:?}", type_errors);
        assert_eq!(
            workspace
                .root_modules()
                .into_iter()
                .map(|module| module.to_slash_path())
                .collect::<Vec<_>>(),
            vec![String::from("Main")]
        );
    }

    #[test]
    fn transitive_local_path_dependencies_work() {
        let root = temp_package_root("transitive-dependency");
        let package_a = root.join("a");
        let package_b = root.join("b");

        write_package(
            &package_b,
            "\
[package]
name = \"b\"
",
            &[(
                "src/B.par",
                "\
export module B

export {
  dec Run : !
}

def Run = !
",
            )],
        );
        write_package(
            &package_a,
            "\
[package]
name = \"a\"

[dependencies]
b = \"../b\"
",
            &[(
                "src/A.par",
                "\
export module A
import @b/B

export {
  dec Run : !
}

def Run = B.Run
",
            )],
        );
        write_package(
            &root,
            "\
[package]
name = \"root\"

[dependencies]
a = \"./a\"
",
            &[(
                "src/Main.par",
                "\
module Main
import @a/A

dec Main : !
def Main = A.Run
",
            )],
        );

        let workspace =
            assemble_workspace(discover_workspace_packages_from_path(&root, None).unwrap())
                .unwrap();
        let (_checked, type_errors) = workspace.type_check();
        assert!(type_errors.is_empty(), "type errors: {:?}", type_errors);
    }

    #[test]
    fn local_dependency_cycle_is_rejected_during_discovery() {
        let root = temp_package_root("cycle");
        let package_a = root.join("a");

        write_package(
            &package_a,
            "\
[package]
name = \"a\"

[dependencies]
root = \"../\"
",
            &[("src/A.par", "module A\n")],
        );
        write_package(
            &root,
            "\
[package]
name = \"root\"

[dependencies]
a = \"./a\"
",
            &[("src/Main.par", "module Main\n")],
        );

        let error = discover_workspace_packages_from_path(&root, None).unwrap_err();
        assert!(matches!(
            error,
            WorkspaceDiscoveryError::PackageCycle { .. }
        ));
    }

    #[test]
    fn missing_local_dependency_path_is_reported_explicitly() {
        let root = temp_package_root("missing-dependency");
        write_package(
            &root,
            "\
[package]
name = \"root\"

[dependencies]
helper = \"./missing-helper\"
",
            &[("src/Main.par", "module Main\n")],
        );

        let error = discover_workspace_packages_from_path(&root, None).unwrap_err();
        assert!(matches!(
            error,
            WorkspaceDiscoveryError::MissingDependencyPackage { alias, .. } if alias == "helper"
        ));
    }

    #[test]
    fn missing_remote_dependencies_are_reported_explicitly() {
        let root = temp_package_root("remote-dependency");
        write_package(
            &root,
            "\
[package]
name = \"root\"

[dependencies]
helper = \"example.com/helper\"
",
            &[("src/Main.par", "module Main\n")],
        );

        let error = discover_workspace_packages_from_path(&root, None).unwrap_err();
        assert!(matches!(
            error,
            WorkspaceDiscoveryError::MissingRemoteDependencyPackage {
                alias,
                dependency,
                path,
                ..
            } if alias == "helper" && dependency == "example.com/helper"
                && path == root
                    .join(DEPENDENCIES_DIRECTORY)
                    .join("example.com")
                    .join("helper")
        ));
    }

    #[test]
    fn materialized_remote_dependency_import_works() {
        let root = temp_package_root("materialized-remote");
        let helper = root
            .join(DEPENDENCIES_DIRECTORY)
            .join("example.com")
            .join("helper");
        write_package(
            &helper,
            "\
[package]
name = \"helper\"
",
            &[(
                "src/Util.par",
                "\
export module Util

export {
  dec Run : !
}

def Run = !
",
            )],
        );
        write_package(
            &root,
            "\
[package]
name = \"root\"

[dependencies]
helper = \"example.com/helper\"
",
            &[(
                "src/Main.par",
                "\
module Main
import @helper/Util

dec Main : !
def Main = Util.Run
",
            )],
        );

        let workspace =
            assemble_workspace(discover_workspace_packages_from_path(&root, None).unwrap())
                .unwrap();
        let (_checked, type_errors) = workspace.type_check();
        assert!(type_errors.is_empty(), "type errors: {:?}", type_errors);
    }

    #[test]
    fn graph_discovery_does_not_parse_broken_sources() {
        let root = temp_package_root("graph-no-parse");
        let helper = root
            .join(DEPENDENCIES_DIRECTORY)
            .join("example.com")
            .join("helper");
        write_package(
            &helper,
            "\
[package]
name = \"helper\"
",
            &[("src/Util.par", "module Util\n")],
        );
        write_package(
            &root,
            "\
[package]
name = \"root\"

[dependencies]
helper = \"example.com/helper\"
",
            &[("src/Main.par", "this is not valid par\n")],
        );

        let graph = PackageGraph::discover_from_path(&root).unwrap();
        assert_eq!(graph.packages.len(), 2);
    }

    #[test]
    fn transitive_remote_dependencies_are_loaded_from_root_dependencies_directory() {
        let root = temp_package_root("transitive-remote");
        let package_a = root
            .join(DEPENDENCIES_DIRECTORY)
            .join("example.com")
            .join("a");
        let package_b = root
            .join(DEPENDENCIES_DIRECTORY)
            .join("example.com")
            .join("b");

        write_package(
            &package_b,
            "\
[package]
name = \"b\"
",
            &[("src/B.par", "export module B\n")],
        );
        write_package(
            &package_a,
            "\
[package]
name = \"a\"

[dependencies]
b = \"example.com/b\"
",
            &[("src/A.par", "export module A\n")],
        );
        write_package(
            &root,
            "\
[package]
name = \"root\"

[dependencies]
a = \"example.com/a\"
",
            &[("src/Main.par", "module Main\n")],
        );

        let graph = PackageGraph::discover_from_path(&root).unwrap();
        assert!(
            graph
                .packages
                .iter()
                .any(|package| package.id == PackageId::Remote(literal!("example.com/b")))
        );
    }

    #[test]
    fn shared_transitive_remote_dependencies_are_discovered_once() {
        let root = temp_package_root("shared-remote");
        let package_a = root
            .join(DEPENDENCIES_DIRECTORY)
            .join("example.com")
            .join("a");
        let package_b = root
            .join(DEPENDENCIES_DIRECTORY)
            .join("example.com")
            .join("b");
        let package_c = root
            .join(DEPENDENCIES_DIRECTORY)
            .join("example.com")
            .join("c");

        write_package(
            &package_b,
            "\
[package]
name = \"b\"
",
            &[("src/B.par", "export module B\n")],
        );
        write_package(
            &package_a,
            "\
[package]
name = \"a\"

[dependencies]
b = \"example.com/b\"
",
            &[("src/A.par", "export module A\n")],
        );
        write_package(
            &package_c,
            "\
[package]
name = \"c\"

[dependencies]
b = \"example.com/b\"
",
            &[("src/C.par", "export module C\n")],
        );
        write_package(
            &root,
            "\
[package]
name = \"root\"

[dependencies]
a = \"example.com/a\"
c = \"example.com/c\"
",
            &[("src/Main.par", "module Main\n")],
        );

        let graph = PackageGraph::discover_from_path(&root).unwrap();
        assert_eq!(
            graph
                .packages
                .iter()
                .filter(|package| package.id == PackageId::Remote(literal!("example.com/b")))
                .count(),
            1
        );
    }

    #[test]
    fn mixed_local_and_remote_package_cycles_are_rejected() {
        let root = temp_package_root("mixed-cycle");
        let package_a = root.join("a");
        let remote = root
            .join(DEPENDENCIES_DIRECTORY)
            .join("example.com")
            .join("b");

        write_package(
            &remote,
            "\
[package]
name = \"b\"

[dependencies]
root = \"../../../\"
",
            &[("src/B.par", "module B\n")],
        );
        write_package(
            &package_a,
            "\
[package]
name = \"a\"

[dependencies]
b = \"example.com/b\"
",
            &[("src/A.par", "module A\n")],
        );
        write_package(
            &root,
            "\
[package]
name = \"root\"

[dependencies]
a = \"./a\"
",
            &[("src/Main.par", "module Main\n")],
        );

        let error = PackageGraph::discover_from_path(&root).unwrap_err();
        assert!(matches!(
            error,
            WorkspaceDiscoveryError::PackageCycle { .. }
        ));
    }

    #[test]
    fn manifest_url_does_not_affect_local_package_identity() {
        let root = temp_package_root("manifest-url-ignored");
        write_package(
            &root,
            "\
[package]
name = \"root\"
url = \"github.com/example/root\"
",
            &[("src/Main.par", "module Main\n")],
        );

        let workspace_packages = discover_workspace_packages_from_path(&root, None).unwrap();
        let canonical_root = canonicalize_path(&root).unwrap();
        assert_eq!(
            workspace_packages.root_package,
            package_id_from_local_path(&canonical_root).unwrap()
        );
    }

    #[test]
    fn import_hover_hides_module_private_items() {
        let main_source = "\
module Main
import Helper
";
        let checked = checked_workspace_from_files(
            "local",
            &[
                (
                    "Helper.par",
                    "\
module Helper

export {
  type Shared = !
  dec Ping : !
}

type Hidden = !
def Ping = external
def Secret: ! = external
",
                ),
                ("Main.par", main_source),
            ],
        );
        let file = FileName::from("local/Main.par");

        let helper_index = main_source.match_indices("Helper").last().unwrap().0;
        let (row, column) = row_and_column(main_source, helper_index);
        let hover = checked.hover_at(&file, row, column).unwrap();
        let signature = checked.render_hover_signature_in_file(&file, &hover);

        assert!(signature.contains("module Helper"));
        assert!(signature.contains("type Helper.Shared"));
        assert!(signature.contains("dec Helper.Ping"));
        assert!(!signature.contains(" = "));
        assert!(!signature.contains(" : "));
        assert!(!signature.contains("Hidden"));
        assert!(!signature.contains("Secret"));
    }
}
