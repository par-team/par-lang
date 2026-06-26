use super::io::IO;
use crate::language_server::data::ToLspPosition;
use crate::package_utils::{SourceLookup, root_module_slash_path};
use crate::workspace_support::{
    ScopedTypeError, WorkspaceBuildError, checked_workspace_from_path,
    checked_workspace_from_single_file,
};
use indexmap::IndexMap;
use lsp_types::{self as lsp, Uri};
use par_core::frontend::{Type, language::GlobalName};
use par_core::source::{FileName, Span};
use par_core::workspace::{
    CheckedWorkspace, SourceOverrides, WorkspaceDiscoveryError, WorkspaceError,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use url::Url;

#[derive(Debug, Clone)]
pub enum CompileError {
    Discovery(WorkspaceDiscoveryError),
    Workspace(WorkspaceError),
    Type {
        error: ScopedTypeError,
        sources: SourceLookup,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum SymbolKey<S> {
    Type(GlobalName<S>),
    Value(GlobalName<S>),
}

pub struct Instance {
    uri: Uri,
    file: FileName,
    dirty: bool,
    checked: Option<Arc<CheckedWorkspace>>,
    errors: Vec<CompileError>,
    io: IO,
}

impl Instance {
    pub fn new(uri: Uri, io: IO) -> Instance {
        Self {
            file: file_name_for_uri(&uri),
            uri,
            dirty: true,
            checked: None,
            errors: Vec::new(),
            io,
        }
    }

    pub fn handle_hover(&self, params: &lsp::HoverParams) -> Option<lsp::Hover> {
        tracing::debug!("Handling hover request with params: {:?}", params);

        let pos = params.text_document_position_params.position;

        let contents = match self.checked.as_ref() {
            Some(checked) => match checked.hover_at(&self.file, pos.line, pos.character) {
                Some(name_info) => lsp::HoverContents::Markup(lsp::MarkupContent {
                    kind: lsp::MarkupKind::Markdown,
                    value: checked.render_hover_markdown_in_file(&self.file, &name_info),
                }),
                _ => return None,
            },
            None => lsp::HoverContents::Markup(lsp::MarkupContent {
                kind: lsp::MarkupKind::PlainText,
                value: "Not compiled".to_string(),
            }),
        };

        let hover = lsp::Hover {
            contents,
            range: None,
        };
        Some(hover)
    }

    /* todo:
    look at C language servers, how they handle split declaration/definition
    look at Rust language servers, what "kind" they use for type aliases & traits
     */
    #[allow(deprecated)] // some types only allow construction using deprecated fields
    pub fn provide_document_symbols(
        &self,
        params: &lsp::DocumentSymbolParams,
    ) -> Option<lsp::DocumentSymbolResponse> {
        tracing::debug!("Handling symbols request with params: {:?}", params);

        let Some(checked) = self.checked.as_ref() else {
            return None;
        };
        let checked_module = checked.checked_module();
        let same_file = |span: &Span| span.file() == Some(self.file.clone());

        let mut symbols = IndexMap::new();

        /* kinds (maybe like this):
        CLASS: choice type
        METHOD: receiving choice branch, trait function
        PROPERTY: general choice branch, trait constant
        ENUM: either type
        INTERFACE: trait
        FUNCTION: value of receiving type
        CONSTANT: value of other type
        OBJECT: value of choice type
        ENUM_MEMBER: either variant
        STRUCT: record
        TYPE_PARAMETER: type alias
         */

        for (name, (span, _, _)) in checked_module.type_defs.globals.as_ref() {
            if !same_file(span) {
                continue;
            }
            if let (Some((name_start, name_end)), Some((start, end))) =
                (name.span.points(), span.points())
            {
                symbols.insert(
                    SymbolKey::Type(name.clone()),
                    lsp::DocumentSymbol {
                        name: checked.render_global_in_file(&self.file, name),
                        detail: None,
                        kind: lsp::SymbolKind::INTERFACE,
                        tags: None,
                        deprecated: None, // must be specified
                        range: lsp::Range {
                            start: start.to_lsp_position(),
                            end: end.to_lsp_position(),
                        },
                        selection_range: lsp::Range {
                            start: name_start.to_lsp_position(),
                            end: name_end.to_lsp_position(),
                        },
                        children: None,
                    },
                );
            }
        }

        for (name, declaration) in &checked_module.declarations {
            if !same_file(&declaration.span) {
                continue;
            }
            let detail = checked.render_type_in_file(&self.file, &declaration.typ, 0);

            if let (Some((name_start, name_end)), Some((start, end))) =
                (name.span.points(), declaration.span.points())
            {
                symbols.insert(
                    SymbolKey::Value(name.clone()),
                    lsp::DocumentSymbol {
                        name: checked.render_global_in_file(&self.file, name),
                        detail: Some(detail),
                        kind: lsp::SymbolKind::FUNCTION,
                        tags: None,
                        deprecated: None, // must be specified
                        range: lsp::Range {
                            start: start.to_lsp_position(),
                            end: end.to_lsp_position(),
                        },
                        selection_range: lsp::Range {
                            start: name_start.to_lsp_position(),
                            end: name_end.to_lsp_position(),
                        },
                        children: None,
                    },
                );
            }
        }

        for (name, (definition, typ)) in &checked_module.definitions {
            if !same_file(&definition.span) {
                continue;
            }
            if let (Some((name_start, name_end)), Some((start, end))) =
                (name.span.points(), definition.span.points())
            {
                let range = lsp::Range {
                    start: start.to_lsp_position(),
                    end: end.to_lsp_position(),
                };
                let selection_range = lsp::Range {
                    start: name_start.to_lsp_position(),
                    end: name_end.to_lsp_position(),
                };
                symbols
                    .entry(SymbolKey::Value(name.clone()))
                    .and_modify(|symbol| {
                        symbol.range = range;
                        symbol.selection_range = selection_range;
                    })
                    .or_insert({
                        let detail = checked.render_type_in_file(&self.file, &typ, 0);

                        lsp::DocumentSymbol {
                            name: checked.render_global_in_file(&self.file, name),
                            detail: Some(detail),
                            kind: lsp::SymbolKind::FUNCTION,
                            tags: None,
                            deprecated: None, // must be specified
                            range,
                            selection_range,
                            children: None,
                        }
                    });
            }
        }

        // todo: fix the bug that causes this
        // the same bug also causes run labels to appear on usages of the name
        for symbol in symbols.values() {
            let range = symbol.range;
            let selection_range = symbol.selection_range;
            let inside = range.start.character <= selection_range.start.character
                && range.start.line <= selection_range.start.line
                && range.end.character >= selection_range.end.character
                && range.end.line >= selection_range.end.line;
            if !inside {
                tracing::error!(
                    "Symbol selection range is not inside the range: {:?}",
                    symbol
                );
            }
        }

        Some(lsp::DocumentSymbolResponse::Nested(
            symbols.into_values().collect(),
        ))
    }

    pub fn provide_code_lenses(&self, params: &lsp::CodeLensParams) -> Option<Vec<lsp::CodeLens>> {
        tracing::debug!("Handling code lens request with params: {:?}", params);

        let Some(checked) = self.checked.as_ref() else {
            return None;
        };

        Some(
            checked
                .checked_module()
                .definitions
                .iter()
                .filter_map(|(name, (definition, typ))| {
                    if definition.span.file() != Some(self.file.clone()) {
                        return None;
                    }

                    let module_path =
                        root_module_slash_path(checked.workspace().root_package(), &name.module)?;
                    let (title, command) =
                        if name.primary.starts_with("Test") && name.primary != "Test" {
                            ("Run Test", "par.runTestCli")
                        } else if matches!(typ, Type::Break(_)) {
                            ("Run", "par.runDefinitionCli")
                        } else {
                            return None;
                        };
                    let (start, _) = definition.span.points()?;
                    Some(lsp::CodeLens {
                        range: lsp::Range {
                            start: start.to_lsp_position(),
                            end: start.to_lsp_position(),
                        },
                        command: Some(lsp::Command {
                            title: title.to_string(),
                            command: command.to_string(),
                            arguments: Some(vec![
                                serde_json::Value::String(self.uri.to_string()),
                                serde_json::Value::String(format!(
                                    "{module_path}.{}",
                                    name.primary
                                )),
                            ]),
                        }),
                        data: None,
                    })
                })
                .collect(),
        )
    }

    pub fn provide_completion(
        &self,
        params: &lsp::CompletionParams,
    ) -> Option<lsp::CompletionResponse> {
        tracing::debug!("Handling completion request with params: {:?}", params);

        let checked = self.checked.as_ref()?;
        let source = self.io.read(&self.uri)?;
        let pos = params.text_document_position.position;
        let items = checked
            .dot_completions_at(&self.file, &source, pos.line, pos.character)
            .into_iter()
            .map(|candidate| {
                let kind = if candidate.is_keyword {
                    lsp::CompletionItemKind::KEYWORD
                } else {
                    lsp::CompletionItemKind::ENUM_MEMBER
                };
                lsp::CompletionItem {
                    label: candidate.label,
                    kind: Some(kind),
                    detail: Some(candidate.detail),
                    insert_text: Some(candidate.insert_text),
                    insert_text_format: Some(lsp::InsertTextFormat::PLAIN_TEXT),
                    ..lsp::CompletionItem::default()
                }
            })
            .collect();

        Some(lsp::CompletionResponse::Array(items))
    }

    pub fn handle_goto_declaration(
        &self,
        params: &lsp::GotoDefinitionParams,
    ) -> Option<lsp::GotoDefinitionResponse> {
        // todo: locals

        tracing::debug!(
            "Handling goto declaration request with params: {:?}",
            params
        );
        let Some(checked) = self.checked.as_ref() else {
            return None;
        };

        let pos = params.text_document_position_params.position;

        let name_info = checked.hover_at(&self.file, pos.line, pos.character)?;

        let decl_span = name_info.decl_span();
        let (start, end) = decl_span.points()?;
        let path = decl_span.file()?;

        Some(lsp::GotoDefinitionResponse::Scalar(lsp::Location {
            uri: file_name_to_uri(&path)?,
            range: lsp::Range {
                start: start.to_lsp_position(),
                end: end.to_lsp_position(),
            },
        }))
    }

    pub fn handle_goto_definition(
        &self,
        params: &lsp::GotoDefinitionParams,
    ) -> Option<lsp::GotoDefinitionResponse> {
        // todo: locals

        tracing::debug!("Handling goto definition request with params: {:?}", params);
        let Some(checked) = self.checked.as_ref() else {
            return None;
        };

        let pos = params.text_document_position_params.position;

        let name_info = checked.hover_at(&self.file, pos.line, pos.character)?;

        let def_span = name_info.def_span();
        let (start, end) = def_span.points()?;
        let path = def_span.file()?;

        Some(lsp::GotoDefinitionResponse::Scalar(lsp::Location {
            uri: file_name_to_uri(&path)?,
            range: lsp::Range {
                start: start.to_lsp_position(),
                end: end.to_lsp_position(),
            },
        }))
    }

    /// Last compile/type errors, if any
    pub fn last_errors(&self) -> &[CompileError] {
        &self.errors
    }

    pub fn run_in_playground(&self, def_name: &str) -> Option<serde_json::Value> {
        tracing::info!("Handling playground request with def_name: {:?}", def_name);
        let Some(checked) = self.checked.as_ref() else {
            return None;
        };

        //TODO: use map indexing
        let Some(_definition) = checked
            .checked_module()
            .definitions
            .iter()
            .find(|(name, _)| checked.render_global_in_file(&self.file, name) == def_name)
        else {
            return None;
        };

        tracing::warn!("Run in playground is not supported!");

        // todo: run

        None
    }

    pub fn compile(&mut self) {
        tracing::info!("Compiling: {:?}", self.uri);
        if !self.dirty {
            tracing::info!("No changes");
            tracing::debug!("No changes to compile");
            return;
        }
        let Some(code) = self.io.read(&self.uri) else {
            self.checked = None;
            self.errors = Vec::new();
            self.dirty = false;
            return;
        };

        let result = {
            let package_result = uri_to_path(&self.uri)
                .map(|path| self.compile_package_with_overlays(&path))
                .unwrap_or_else(|| {
                    Err(CompileError::Discovery(
                        WorkspaceDiscoveryError::PackageRootNotFound {
                            start: PathBuf::from(self.uri.as_str()),
                        },
                    ))
                });

            match package_result {
                Ok(result) => Ok(result),
                Err(CompileError::Discovery(WorkspaceDiscoveryError::PackageRootNotFound {
                    ..
                })) => self.compile_single_file(&code),
                Err(error) => Err(error),
            }
        };

        match result {
            Ok((checked, errors)) => {
                self.checked = Some(checked);
                self.errors = errors;
            }
            Err(error) => {
                self.errors = vec![error];
            }
        }
        tracing::info!("Compiled!");
        // reset dirty flag after successful compile attempt
        self.dirty = false;
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn compile_single_file(
        &self,
        code: &str,
    ) -> Result<(Arc<CheckedWorkspace>, Vec<CompileError>), CompileError> {
        let file_path = uri_to_path(&self.uri).unwrap_or_else(|| PathBuf::from("LspBuffer.par"));
        let build = checked_workspace_from_single_file(&file_path, "LspBuffer.par", code)
            .map_err(map_workspace_build_error)?;
        Ok(build_compile_result(build))
    }

    fn compile_package_with_overlays(
        &self,
        file_path: &Path,
    ) -> Result<(Arc<CheckedWorkspace>, Vec<CompileError>), CompileError> {
        let overlay_sources: SourceOverrides = self
            .io
            .snapshot()
            .into_iter()
            .filter_map(|(uri, source)| uri_to_path(&uri).map(|path| (path, source)))
            .collect();

        let build = checked_workspace_from_path(file_path, Some(&overlay_sources))
            .map_err(map_workspace_build_error)?;
        Ok(build_compile_result(build))
    }
}

fn build_compile_result(
    build: crate::workspace_support::CheckedWorkspaceBuild,
) -> (Arc<CheckedWorkspace>, Vec<CompileError>) {
    let sources = build.sources.clone();
    let errors = build
        .type_errors
        .into_iter()
        .map(|error| CompileError::Type {
            error,
            sources: sources.clone(),
        })
        .collect();
    (Arc::new(build.checked), errors)
}

fn map_workspace_build_error(error: WorkspaceBuildError) -> CompileError {
    match error {
        WorkspaceBuildError::Discovery(error) => CompileError::Discovery(error),
        WorkspaceBuildError::Workspace(error) => CompileError::Workspace(error),
    }
}

fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let url = Url::parse(uri.as_str()).ok()?;
    if url.scheme() != "file" {
        return None;
    }
    url.to_file_path().ok()
}

fn file_name_for_uri(uri: &Uri) -> FileName {
    uri_to_path(uri)
        .map(FileName::from)
        .unwrap_or_else(|| uri.as_str().into())
}

fn file_name_to_uri(file: &FileName) -> Option<Uri> {
    let path = PathBuf::from(file.0.as_str());
    if path.is_absolute() {
        return Url::from_file_path(path)
            .ok()
            .and_then(|url| url.as_str().parse().ok());
    }
    if let Ok(uri) = file.0.as_str().parse() {
        return Some(uri);
    }
    let path = PathBuf::from(file.0.as_str());
    Url::from_file_path(path)
        .ok()
        .and_then(|url| url.as_str().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language_server::feedback::diagnostic_for_error;
    use std::collections::HashMap;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_package(files: &[(&str, &str)]) -> (PathBuf, HashMap<String, Uri>) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("par-lang-lsp-{unique}"));
        fs::create_dir_all(root.join("src")).expect("failed to create temp package");
        fs::write(root.join("Par.toml"), "[package]\nname = \"tmp\"\n")
            .expect("failed to write manifest");

        let mut uris = HashMap::new();
        for (relative_path, source) in files {
            let path = root.join(relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("failed to create temp source directory");
            }
            fs::write(&path, source).expect("failed to write temp source file");
            let uri = Url::from_file_path(&path)
                .expect("temp path should convert to file URL")
                .as_str()
                .parse()
                .expect("file URL should parse as URI");
            uris.insert((*relative_path).to_string(), uri);
        }

        (root, uris)
    }

    #[test]
    fn diagnostics_target_the_file_that_owns_the_error() {
        let (root, uris) = temp_package(&[
            ("src/Main.par", "module Main\n\ndef Main = 0\n"),
            ("src/Other.par", "module Other\n\ndef Broken = (\n"),
        ]);
        let main_uri = uris["src/Main.par"].clone();
        let other_uri = uris["src/Other.par"].clone();

        let mut io = IO::new();
        io.update_file(
            &main_uri,
            fs::read_to_string(root.join("src/Main.par")).expect("failed to read Main.par"),
        );

        let mut instance = Instance::new(main_uri.clone(), io);
        instance.compile();

        let errors = instance.last_errors();
        assert!(!errors.is_empty(), "compile should fail");
        let (diagnostic_uri, diagnostic) = diagnostic_for_error(&errors[0], &main_uri);
        assert_eq!(diagnostic_uri, other_uri);
        assert_eq!(diagnostic.range.start.line, 2);
    }

    #[test]
    fn completion_uses_last_good_checked_workspace() {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                let valid_source = "\
module Main

type Client = choice {
  .close => !,
  .next => !,
}

def ClientValue : Client = external
def Main : Client = ClientValue
";
                let (root, uris) = temp_package(&[("src/Main.par", valid_source)]);
                let main_uri = uris["src/Main.par"].clone();

                let mut io = IO::new();
                io.update_file(&main_uri, valid_source.to_string());

                let mut instance = Instance::new(main_uri.clone(), io.clone());
                instance.compile();
                assert!(instance.checked.is_some(), "valid source should compile");

                for (live_replacement, marker) in [
                    ("ClientValue.\n", "ClientValue."),
                    ("ClientValue.cl\n", "ClientValue.cl"),
                ] {
                    let live_source = valid_source.replace("ClientValue\n", live_replacement);
                    io.update_file(&main_uri, live_source.clone());
                    instance.mark_dirty();
                    instance.compile();
                    assert!(!instance.last_errors().is_empty());
                    assert!(instance.checked.is_some());

                    let response = instance
                        .provide_completion(&lsp::CompletionParams {
                            text_document_position: lsp::TextDocumentPositionParams {
                                text_document: lsp::TextDocumentIdentifier {
                                    uri: main_uri.clone(),
                                },
                                position: lsp::Position {
                                    line: 8,
                                    character: "def Main : Client = ".len() as u32
                                        + marker.len() as u32,
                                },
                            },
                            work_done_progress_params: Default::default(),
                            partial_result_params: Default::default(),
                            context: None,
                        })
                        .expect("completion response");
                    let lsp::CompletionResponse::Array(items) = response else {
                        panic!("expected completion array");
                    };

                    assert!(items.iter().any(|item| item.label == "close"));
                    assert!(items.iter().any(|item| item.label == "next"));
                }

                fs::remove_dir_all(root).ok();
            })
            .expect("failed to spawn large-stack test thread")
            .join()
            .expect("completion test panicked");
    }
}
