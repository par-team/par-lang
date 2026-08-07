use crate::language_server::instance::CompileError;
use lsp_types::{self as lsp, Uri};
use par_core::source::{FileName, Span};
use par_core::workspace::{PackageLoadError, WorkspaceDiscoveryError, WorkspaceError};
use std::collections::HashMap;
use std::path::Path;
use url::Url;

pub struct Feedback {
    diagnostics: HashMap<Uri, Vec<lsp::Diagnostic>>,
}

impl Feedback {
    pub fn new() -> Feedback {
        Self {
            diagnostics: HashMap::new(),
        }
    }

    pub fn diagnostics(&self) -> &HashMap<Uri, Vec<lsp::Diagnostic>> {
        &self.diagnostics
    }

    pub fn add_diagnostic(&mut self, uri: Uri, diagnostic: lsp::Diagnostic) {
        self.diagnostics.entry(uri).or_default().push(diagnostic);
    }
}

pub struct FeedbackBookKeeper {
    feedback: Feedback,
}

impl FeedbackBookKeeper {
    pub fn new() -> FeedbackBookKeeper {
        Self {
            feedback: Feedback::new(),
        }
    }

    /// The last feedback with empty diagnostics
    /// for all URIs, so that the client can clear
    pub fn cleanup(&mut self) -> &mut Feedback {
        let feedback = Feedback::new();
        let last_feedback = std::mem::replace(&mut self.feedback, feedback);
        for (uri, diagnostics) in last_feedback.diagnostics.into_iter() {
            if !diagnostics.is_empty() {
                self.feedback.diagnostics.entry(uri).or_default();
            }
        }
        &mut self.feedback
    }

    pub fn diagnostics(&self) -> &HashMap<Uri, Vec<lsp::Diagnostic>> {
        self.feedback.diagnostics()
    }
}

pub fn diagnostic_for_error(err: &CompileError, fallback_uri: &Uri) -> (Uri, lsp::Diagnostic) {
    let (span, message, _related_spans) = match err {
        CompileError::Type { error, sources } => {
            let (span, related_span) = error.error.spans();
            (
                span,
                strip_ansi_underlining(format!("{:?}", error.to_report(sources))),
                related_span.into_iter().collect(),
            )
        }
        CompileError::Discovery(error) => {
            let (span, related_spans) = error.spans();
            (span, error.to_string(), related_spans)
        }
        CompileError::Workspace(error) => {
            let (span, related_spans) = error.spans();
            (span, error.to_string(), related_spans)
        }
    };
    let severity = match err {
        CompileError::Type { error, .. } if error.error.is_warning() => {
            lsp::DiagnosticSeverity::WARNING
        }
        _ => lsp::DiagnosticSeverity::ERROR,
    };
    (
        uri_for_error(err).unwrap_or_else(|| fallback_uri.clone()),
        lsp::Diagnostic {
            range: span_to_lsp_range(&span),
            severity: Some(severity),
            code: None,
            code_description: None,
            source: None,
            message,
            related_information: None, // todo
            tags: None,
            data: None,
        },
    )
}

fn strip_ansi_underlining(message: String) -> String {
    message.replace("\x1b[4m", "").replace("\x1b[24m", "")
}

fn span_to_lsp_range(span: &Span) -> lsp::Range {
    match span {
        Span::None => lsp::Range {
            start: lsp::Position {
                line: 0,
                character: 0,
            },
            end: lsp::Position {
                line: 0,
                character: 0,
            },
        },
        Span::At { start, end, .. } => lsp::Range {
            start: lsp::Position {
                line: start.row as u32,
                character: start.column as u32,
            },
            end: lsp::Position {
                line: end.row as u32,
                character: end.column as u32,
            },
        },
    }
}

fn uri_for_error(err: &CompileError) -> Option<Uri> {
    match err {
        CompileError::Type { error, .. } => {
            let (span, _) = error.error.spans();
            uri_for_span(&span)
        }
        CompileError::Discovery(error) => uri_for_discovery_error(error),
        CompileError::Workspace(WorkspaceError::LowerError { file, .. }) => file_name_to_uri(file),
        CompileError::Workspace(WorkspaceError::UnknownDependency { span, .. })
        | CompileError::Workspace(WorkspaceError::ImportedModuleNotFound { span, .. })
        | CompileError::Workspace(WorkspaceError::DuplicateImportAlias { span, .. })
        | CompileError::Workspace(WorkspaceError::BindingNameConflictsWithImportAlias {
            span,
            ..
        })
        | CompileError::Workspace(WorkspaceError::UnknownModuleQualifier { span, .. })
        | CompileError::Workspace(WorkspaceError::QualifiedCurrentModuleReference {
            span, ..
        }) => uri_for_span(span),
        CompileError::Workspace(WorkspaceError::UnattachedExternalModule { .. }) => None,
    }
}

fn uri_for_discovery_error(error: &WorkspaceDiscoveryError) -> Option<Uri> {
    match error {
        WorkspaceDiscoveryError::Load(error) => uri_for_load_error(error),
        _ => None,
    }
}

fn uri_for_load_error(error: &PackageLoadError) -> Option<Uri> {
    match error {
        PackageLoadError::ParseError { file, .. }
        | PackageLoadError::MissingModuleDeclaration { file }
        | PackageLoadError::FileNameModuleMismatch { file, .. }
        | PackageLoadError::ConflictingModuleNameCasing {
            first_file: file, ..
        } => file_name_to_uri(file),
        PackageLoadError::DirectoryReadError { .. }
        | PackageLoadError::FileReadError { .. }
        | PackageLoadError::InvalidSourceFilePath { .. }
        | PackageLoadError::InvalidSourceFileName { .. } => None,
    }
}

fn uri_for_span(span: &Span) -> Option<Uri> {
    span.file().and_then(|file| file_name_to_uri(&file))
}

fn file_name_to_uri(file: &FileName) -> Option<Uri> {
    let path = Path::new(file.0.as_str());
    if path.is_absolute() {
        return path_to_uri(path);
    }
    file.0.as_str().parse().ok()
}

fn path_to_uri(path: &Path) -> Option<Uri> {
    Url::from_file_path(path)
        .ok()
        .and_then(|url| url.as_str().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::strip_ansi_underlining;

    #[test]
    fn ansi_underlining_is_removed_from_diagnostic_messages() {
        assert_eq!(
            strip_ansi_underlining("before \x1b[4mhighlighted\x1b[24m after".to_string()),
            "before highlighted after",
        );
    }
}
