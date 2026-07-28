use std::path::{Path, PathBuf};

use crate::package_utils::{SourceLookup, source_for_type_error};
use arcstr::literal;
use par_builtin::inject_builtin_packages;
use par_core::frontend::{TypeError, language::Universal};
use par_core::runtime::{Compiled, RuntimeCompilerError};
use par_core::source::FileName;
use par_core::workspace::{
    CheckedWorkspace, FileImportScope, LoadedPackageFile, ParsedPackage, SourceOverrides,
    Workspace, WorkspaceDiscoveryError, WorkspacePackage, WorkspacePackages, assemble_workspace,
    discover_workspace_packages_from_path, parse_loaded_files,
};
use par_runtime::linker::{Linked, Unlinked};
use par_runtime::pkgid::PackageId;

#[derive(Debug, Clone)]
pub(crate) enum WorkspaceBuildError {
    Discovery(WorkspaceDiscoveryError),
    Workspace(par_core::workspace::WorkspaceError),
}

#[derive(Debug, Clone)]
pub(crate) struct ScopedTypeError {
    pub error: TypeError<Universal>,
    pub file_scope: Option<FileImportScope<Universal>>,
}

impl ScopedTypeError {
    fn from_workspace(workspace: &Workspace, error: TypeError<Universal>) -> Self {
        Self {
            file_scope: error
                .spans()
                .0
                .file()
                .and_then(|file| workspace.import_scope(&file).cloned()),
            error,
        }
    }

    pub(crate) fn to_report(&self, sources: &SourceLookup) -> miette::Report {
        self.error.to_report(
            source_for_type_error(&self.error, sources),
            self.file_scope.as_ref(),
        )
    }
}

#[derive(Clone)]
pub(crate) struct CheckedWorkspaceBuild {
    pub checked: CheckedWorkspace,
    pub sources: SourceLookup,
    pub type_errors: Vec<ScopedTypeError>,
}

impl CheckedWorkspaceBuild {
    fn from_workspace(workspace: Workspace) -> Self {
        let sources = workspace.sources().clone();
        let (checked, type_errors) = workspace.type_check();
        Self {
            checked,
            sources,
            type_errors: type_errors
                .into_iter()
                .map(|error| ScopedTypeError::from_workspace(&workspace, error))
                .collect(),
        }
    }

    pub(crate) fn warnings(&self) -> impl Iterator<Item = &ScopedTypeError> {
        self.type_errors.iter().filter(|e| e.error.is_warning())
    }

    pub(crate) fn compile_unlinked(
        self,
        max_interactions: u32,
    ) -> Result<
        (CheckedWorkspace, Compiled<Unlinked>, SourceLookup),
        (CheckedWorkspace, RuntimeCompilerError),
    > {
        let Self {
            checked,
            sources,
            type_errors: _,
        } = self;
        match checked.compile_runtime(max_interactions) {
            Ok(compiled) => Ok((checked, compiled, sources)),
            Err(error) => Err((checked, error)),
        }
    }

    pub(crate) fn compile_linked(
        self,
        max_interactions: u32,
    ) -> Result<
        (CheckedWorkspace, Compiled<Linked>, SourceLookup),
        (CheckedWorkspace, RuntimeCompilerError),
    > {
        let (checked, compiled, sources) = self.compile_unlinked(max_interactions)?;
        match compiled.link() {
            Ok(compiled) => Ok((checked, compiled, sources)),
            Err(error) => Err((checked, error)),
        }
    }
}

pub fn default_workspace_packages_from_path(
    start: impl AsRef<Path>,
    overrides: Option<&SourceOverrides>,
) -> Result<WorkspacePackages, WorkspaceDiscoveryError> {
    let mut discovered = discover_workspace_packages_from_path(start, overrides)?;
    inject_builtin_packages(&mut discovered)?;
    Ok(discovered)
}

pub fn default_workspace_packages_from_parsed(
    root_package: PackageId,
    local: ParsedPackage,
) -> WorkspacePackages {
    let mut packages = WorkspacePackages {
        root_package: root_package.clone(),
        packages: vec![WorkspacePackage::new(root_package, local)],
    };
    inject_builtin_packages(&mut packages)
        .expect("synthetic workspace should not conflict with builtin aliases");
    packages
}

pub fn assemble_default_workspace(
    workspace_packages: WorkspacePackages,
) -> Result<Workspace, par_core::workspace::WorkspaceError> {
    assemble_workspace(workspace_packages)
}

pub(crate) fn checked_workspace_from_path(
    start: impl AsRef<Path>,
    overrides: Option<&SourceOverrides>,
) -> Result<CheckedWorkspaceBuild, WorkspaceBuildError> {
    let packages = default_workspace_packages_from_path(start, overrides)
        .map_err(WorkspaceBuildError::Discovery)?;
    let workspace = assemble_default_workspace(packages).map_err(WorkspaceBuildError::Workspace)?;
    Ok(CheckedWorkspaceBuild::from_workspace(workspace))
}

fn checked_workspace_from_parsed(
    root_package: PackageId,
    parsed: ParsedPackage,
) -> Result<CheckedWorkspaceBuild, WorkspaceBuildError> {
    let workspace_packages = default_workspace_packages_from_parsed(root_package, parsed);
    let workspace =
        assemble_default_workspace(workspace_packages).map_err(WorkspaceBuildError::Workspace)?;
    Ok(CheckedWorkspaceBuild::from_workspace(workspace))
}

pub(crate) fn checked_workspace_from_loaded_package(
    files: Vec<LoadedPackageFile>,
    root_package: PackageId,
) -> Result<CheckedWorkspaceBuild, WorkspaceBuildError> {
    let parsed = parse_loaded_files(files)
        .map_err(|error| WorkspaceBuildError::Discovery(WorkspaceDiscoveryError::Load(error)))?;
    checked_workspace_from_parsed(root_package, parsed)
}

pub(crate) fn checked_workspace_from_single_file(
    file_path: &Path,
    fallback_file_name: &str,
    source: &str,
) -> Result<CheckedWorkspaceBuild, WorkspaceBuildError> {
    let relative_path_from_src = file_path
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(fallback_file_name));
    checked_workspace_from_loaded_package(
        vec![LoadedPackageFile {
            name: FileName::from(file_path),
            relative_path_from_src,
            source: source.to_owned(),
        }],
        PackageId::Special(literal!("__synthetic__")),
    )
}
