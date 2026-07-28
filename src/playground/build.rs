use std::fmt::Write;
#[cfg(not(target_family = "wasm"))]
use std::path::Path;
use std::sync::Arc;

use crate::package_utils::SourceLookup;
#[cfg(not(target_family = "wasm"))]
use crate::workspace_support::checked_workspace_from_path;
use crate::workspace_support::{
    CheckedWorkspaceBuild, ScopedTypeError, WorkspaceBuildError,
    checked_workspace_from_loaded_package,
};
use par_core::frontend::{Definition, DefinitionBody, language::Universal, process::Expression};
use par_core::source::FileName;
#[cfg(not(target_family = "wasm"))]
use par_core::workspace::SourceOverrides;
use par_core::{
    runtime::{Compiled, RuntimeCompilerError},
    workspace::{CheckedWorkspace, LoadedPackageFile, WorkspaceDiscoveryError, WorkspaceError},
};
use par_runtime::linker::Linked;
use par_runtime::pkgid::PackageId;

type LoweredDefinition = Definition<Arc<Expression<(), Universal>>, Universal>;

#[derive(Clone)]
pub(super) enum BuildError {
    Discovery(WorkspaceDiscoveryError),
    Workspace(WorkspaceError),
    Type {
        errors: Vec<ScopedTypeError>,
        sources: SourceLookup,
    },
    InetCompile(RuntimeCompilerError),
}

impl BuildError {
    pub(super) fn display(&self, code: Arc<str>) -> String {
        match self {
            Self::Discovery(error) => error.to_string(),
            Self::Workspace(error) => error.to_string(),
            Self::Type { errors, sources } => errors
                .iter()
                .map(|error| format!("{:?}", error.to_report(sources)))
                .collect::<Vec<_>>()
                .join("\n"),
            Self::InetCompile(error) => format!("inet compilation error: {}", error.display(&code)),
        }
    }
}

#[derive(Clone)]
pub(super) enum BuildResult {
    None,
    DiscoveryError {
        error: WorkspaceDiscoveryError,
    },
    WorkspaceError {
        error: WorkspaceError,
    },
    TypeError {
        pretty: String,
        checked: Arc<CheckedWorkspace>,
        errors: Vec<ScopedTypeError>,
        sources: SourceLookup,
    },
    InetError {
        pretty: String,
        checked: Arc<CheckedWorkspace>,
        error: RuntimeCompilerError,
    },
    Ok {
        pretty: String,
        checked: Arc<CheckedWorkspace>,
        rt_compiled: Compiled<Linked>,
    },
}

impl BuildResult {
    pub(super) fn error(&self) -> Option<BuildError> {
        match self {
            Self::None => None,
            Self::DiscoveryError { error } => Some(BuildError::Discovery(error.clone())),
            Self::WorkspaceError { error } => Some(BuildError::Workspace(error.clone())),
            Self::TypeError {
                errors, sources, ..
            } => Some(BuildError::Type {
                errors: errors.clone(),
                sources: sources.clone(),
            }),
            Self::InetError { error, .. } => Some(BuildError::InetCompile(error.clone())),
            Self::Ok { .. } => None,
        }
    }

    pub(super) fn pretty(&self) -> Option<&str> {
        match self {
            Self::TypeError { pretty, .. }
            | Self::InetError { pretty, .. }
            | Self::Ok { pretty, .. } => Some(pretty),
            Self::None | Self::DiscoveryError { .. } | Self::WorkspaceError { .. } => None,
        }
    }

    pub(super) fn pretty_for_file(&self, file: &FileName) -> Option<String> {
        let checked = self.checked()?;
        let Some(scope) = checked.workspace().import_scope(file) else {
            return self.pretty().map(str::to_owned);
        };

        Some(
            checked
                .workspace()
                .lowered_module()
                .definitions
                .iter()
                .filter(|definition| definition.name.module == scope.current_module)
                .map(format_definition)
                .collect(),
        )
    }

    pub(super) fn checked(&self) -> Option<Arc<CheckedWorkspace>> {
        match self {
            Self::TypeError { checked, .. }
            | Self::InetError { checked, .. }
            | Self::Ok { checked, .. } => Some(Arc::clone(checked)),
            Self::None | Self::DiscoveryError { .. } | Self::WorkspaceError { .. } => None,
        }
    }

    pub(super) fn rt_compiled(&self) -> Option<&Compiled<Linked>> {
        match self {
            Self::Ok { rt_compiled, .. } => Some(rt_compiled),
            Self::None
            | Self::DiscoveryError { .. }
            | Self::WorkspaceError { .. }
            | Self::TypeError { .. }
            | Self::InetError { .. } => None,
        }
    }

    pub(super) fn from_loaded_package(
        files: Vec<LoadedPackageFile>,
        root_package: PackageId,
        max_interactions: u32,
    ) -> Self {
        Self::from_workspace_build_result(
            checked_workspace_from_loaded_package(files, root_package),
            max_interactions,
        )
    }

    #[cfg(not(target_family = "wasm"))]
    pub(super) fn from_package_with_overrides(
        active_file_path: &Path,
        overrides: SourceOverrides,
        max_interactions: u32,
    ) -> Self {
        Self::from_workspace_build_result(
            checked_workspace_from_path(active_file_path, Some(&overrides)),
            max_interactions,
        )
    }

    fn from_workspace_build_result(
        result: Result<CheckedWorkspaceBuild, WorkspaceBuildError>,
        max_interactions: u32,
    ) -> Self {
        match result {
            Ok(build) => Self::from_checked_build(build, max_interactions),
            Err(WorkspaceBuildError::Discovery(error)) => Self::DiscoveryError { error },
            Err(WorkspaceBuildError::Workspace(error)) => Self::WorkspaceError { error },
        }
    }

    fn from_checked_build(build: CheckedWorkspaceBuild, max_interactions: u32) -> Self {
        let pretty = build
            .checked
            .workspace()
            .lowered_module()
            .definitions
            .iter()
            .map(format_definition)
            .collect();

        if build.type_errors.iter().any(|e| !e.error.is_warning()) {
            return Self::TypeError {
                pretty,
                checked: Arc::new(build.checked),
                errors: build.type_errors,
                sources: build.sources,
            };
        }
        let (checked, rt_compiled, _) = match build.compile_linked(max_interactions) {
            Ok(build) => build,
            Err((checked, error)) => {
                return Self::InetError {
                    pretty,
                    checked: Arc::new(checked),
                    error,
                };
            }
        };
        Self::Ok {
            pretty,
            checked: Arc::new(checked),
            rt_compiled,
        }
    }
}

fn format_definition(
    Definition {
        span: _,
        name,
        body,
    }: &LoweredDefinition,
) -> String {
    let mut buf = String::new();
    write!(&mut buf, "def {name} = ").expect("write failed");
    match body {
        DefinitionBody::Par(expr) => {
            expr.pretty(&mut buf, 0).expect("write failed");
        }
        DefinitionBody::External(_) => {
            write!(&mut buf, "<external>").expect("write failed");
        }
    }
    write!(&mut buf, "\n\n").expect("write failed");
    buf
}
