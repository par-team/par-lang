use super::io::IO;
use crate::language_server::ext::{GetBuiltinFileContent, GetBuiltinFileContentParams};
use crate::language_server::feedback::{FeedbackBookKeeper, diagnostic_for_error};
use crate::language_server::instance::Instance;
use lsp_server::{Connection, ErrorCode};
use lsp_types::notification::DidSaveTextDocument;
use lsp_types::request::{
    CodeLensRequest, Completion, DocumentSymbolRequest, ExecuteCommand, GotoDeclaration,
    GotoDefinition,
};
use lsp_types::{self as lsp, InitializeParams, Uri};
use par_builtin::get_builtin_source;
use std::collections::HashMap;
use std::str::FromStr;

type Instances = HashMap<Uri, Instance>;

pub struct LanguageServer<'c> {
    connection: &'c Connection,
    _initialize_params: InitializeParams,
    instances: Instances,
    feedback: FeedbackBookKeeper,
    io: IO,
}

impl<'c> LanguageServer<'c> {
    pub fn new(connection: &'c Connection) -> LanguageServer<'c> {
        let initialize_params = initialize_lsp(connection);
        Self {
            connection,
            _initialize_params: initialize_params,
            instances: HashMap::new(),
            feedback: FeedbackBookKeeper::new(),
            io: IO::new(),
        }
    }

    pub fn run(&mut self) {
        loop {
            let message = self
                .connection
                .receiver
                .recv()
                .expect("Receiving message failed");
            tracing::debug!("Received message: {:?}", message);

            match message {
                lsp_server::Message::Request(r) if self.should_shutdown(&r) => break,
                lsp_server::Message::Request(r) => self.handle_request(r),
                lsp_server::Message::Response(r) => self.handle_response(r),
                lsp_server::Message::Notification(n) => self.handle_notification(n),
            }
        }
    }

    fn handle_request(&mut self, request: lsp_server::Request) {
        use lsp::request::{HoverRequest, Request};

        let request_id = request.id.clone();
        let method = request.method.as_str();
        let response = match method {
            HoverRequest::METHOD => {
                let params = extract_request::<HoverRequest>(request);
                self.handle_request_instance(
                    request_id,
                    &params.text_document_position_params.text_document.uri,
                    |instance| instance.handle_hover(&params),
                )
            }
            DocumentSymbolRequest::METHOD => {
                let params = extract_request::<DocumentSymbolRequest>(request);
                self.handle_request_instance(request_id, &params.text_document.uri, |instance| {
                    instance.provide_document_symbols(&params)
                })
            }
            CodeLensRequest::METHOD => {
                let params = extract_request::<CodeLensRequest>(request);
                self.handle_request_instance(request_id, &params.text_document.uri, |instance| {
                    instance.provide_code_lenses(&params)
                })
            }
            Completion::METHOD => {
                let params = extract_request::<Completion>(request);
                self.handle_request_instance(
                    request_id,
                    &params.text_document_position.text_document.uri,
                    |instance| instance.provide_completion(&params),
                )
            }
            GotoDeclaration::METHOD => {
                let params = extract_request::<GotoDeclaration>(request);
                self.handle_request_instance(
                    request_id,
                    &params.text_document_position_params.text_document.uri,
                    |instance| instance.handle_goto_declaration(&params),
                )
            }
            GotoDefinition::METHOD => {
                let params = extract_request::<GotoDefinition>(request);
                self.handle_request_instance(
                    request_id,
                    &params.text_document_position_params.text_document.uri,
                    |instance| instance.handle_goto_definition(&params),
                )
            }
            ExecuteCommand::METHOD => {
                let params = extract_request::<ExecuteCommand>(request);
                match params.command.as_str() {
                    "run" => {
                        let Some(uri_str) = params.arguments.get(0).and_then(|v| v.as_str()) else {
                            return;
                        };
                        let Some(def_name) = params.arguments.get(1).and_then(|v| v.as_str())
                        else {
                            return;
                        };
                        let Ok(uri) = Uri::from_str(uri_str) else {
                            return;
                        };
                        self.handle_request_instance(request_id, &uri, |instance| {
                            instance.run_in_playground(def_name)
                        })
                    }
                    _ => {
                        tracing::warn!("Unhandled command: {:?}", params);
                        return;
                    }
                }
            }
            GetBuiltinFileContent::METHOD => {
                let GetBuiltinFileContentParams { builtin_path } =
                    extract_request::<GetBuiltinFileContent>(request);

                if let Some(content) = get_builtin_source(&builtin_path) {
                    lsp_server::Response::new_ok(request_id, content)
                } else {
                    lsp_server::Response::new_err(
                        request_id,
                        ErrorCode::InvalidRequest as i32,
                        format!("unknown builtin {builtin_path}"),
                    )
                }
            }
            _ => {
                tracing::warn!("Unhandled request: {:?}", request);
                return;
            }
        };

        tracing::debug!("Responding {:?}", response);
        self.connection
            .sender
            .send(lsp_server::Message::Response(response))
            .expect("Sending response failed");

        self.publish_feedback();
    }

    fn handle_response(&self, response: lsp_server::Response) {
        tracing::warn!("Unhandled response: {:?}", response);
    }

    fn handle_notification(&mut self, notification: lsp_server::Notification) {
        use lsp::notification::{DidChangeTextDocument, DidOpenTextDocument, Notification};

        let method = notification.method.as_str();
        match method {
            DidOpenTextDocument::METHOD => {
                let params = extract_notification::<DidOpenTextDocument>(notification);
                let uri = &params.text_document.uri;
                self.cache_file(uri, params.text_document.text);
                self.compile(uri);
                self.publish_feedback();
            }
            DidChangeTextDocument::METHOD => {
                let params = extract_notification::<DidChangeTextDocument>(notification);
                if let Some(last_change) = params.content_changes.into_iter().last() {
                    let uri = &params.text_document.uri;
                    self.cache_file(uri, last_change.text);
                    // compile on change to keep diagnostics up-to-date
                    self.compile(uri);
                    self.publish_feedback();
                }
            }
            DidSaveTextDocument::METHOD => {
                let params = extract_notification::<DidSaveTextDocument>(notification);
                let uri = &params.text_document.uri;
                self.compile(uri);
                self.publish_feedback();
            }
            // todo: handle closing
            _ => {
                tracing::warn!("Unhandled notification: {:?}", notification);
                return;
            }
        }
    }

    fn should_shutdown(&self, request: &lsp_server::Request) -> bool {
        self.connection
            .handle_shutdown(request)
            .expect("Protocol error while handling shutdown")
    }

    fn handle_request_instance<R: serde::Serialize>(
        &mut self,
        id: lsp_server::RequestId,
        uri: &Uri,
        handler: impl FnOnce(&mut Instance) -> R,
    ) -> lsp_server::Response {
        self.compile(uri);
        let instance = self.instance_for(uri);
        let response = handler(instance);
        lsp_server::Response::new_ok(id, response)
    }

    fn publish_feedback(&mut self) {
        use lsp::notification::{Notification, PublishDiagnostics};

        for (uri, diagnostics) in self.feedback.diagnostics() {
            let params = lsp_types::PublishDiagnosticsParams {
                uri: uri.clone(),
                diagnostics: diagnostics.clone(),
                version: None,
            };
            let notification = lsp_server::Notification {
                method: PublishDiagnostics::METHOD.to_string(),
                params: serde_json::to_value(params).unwrap(),
            };

            self.connection
                .sender
                .send(lsp_server::Message::Notification(notification))
                .expect("Sending notification failed");
        }
    }

    fn compile(&mut self, uri: &Uri) {
        // Compile and capture last error while holding the instance borrow
        let errs = {
            let instance = self.instance_for(uri);
            instance.compile();
            instance.last_errors().to_vec()
        };

        // Now update feedback/diagnostics
        let feedback = self.feedback.cleanup();
        for err in errs {
            let (diagnostic_uri, diagnostic) = diagnostic_for_error(&err, uri);
            feedback.add_diagnostic(diagnostic_uri, diagnostic);
        }
    }

    fn cache_file(&mut self, uri: &Uri, text: String) {
        tracing::info!("Caching file: {:?}", uri);
        self.io.update_file(uri, text);
        for instance in self.instances.values_mut() {
            instance.mark_dirty();
        }
    }

    fn instance_for(&mut self, uri: &Uri) -> &mut Instance {
        self.instances
            .entry(uri.clone())
            .or_insert_with(|| Instance::new(uri.clone(), self.io.clone()))
    }
}

fn initialize_lsp(connection: &Connection) -> InitializeParams {
    let server_capabilities = lsp::ServerCapabilities {
        text_document_sync: Some(lsp::TextDocumentSyncCapability::Kind(
            lsp::TextDocumentSyncKind::FULL,
        )),
        hover_provider: Some(lsp::HoverProviderCapability::Simple(true)),
        document_symbol_provider: Some(lsp::OneOf::Left(true)),
        code_lens_provider: Some(lsp::CodeLensOptions {
            resolve_provider: Some(false),
        }),
        completion_provider: Some(lsp::CompletionOptions {
            resolve_provider: Some(false),
            trigger_characters: Some(vec![".".to_owned()]),
            ..lsp::CompletionOptions::default()
        }),
        declaration_provider: Some(lsp::DeclarationCapability::Simple(true)),
        definition_provider: Some(lsp::OneOf::Left(true)),
        execute_command_provider: Some(lsp::ExecuteCommandOptions {
            commands: vec!["run".to_owned()],
            work_done_progress_options: Default::default(),
        }),

        ..lsp::ServerCapabilities::default()
    };
    let server_capabilities_json = serde_json::to_value(&server_capabilities).unwrap();
    let result_json = connection
        .initialize(server_capabilities_json)
        .expect("Initializing LSP failed");
    let result: InitializeParams = serde_json::from_value(result_json).unwrap();
    tracing::debug!("Initialized LSP with params: {:?}", result);
    result
}

fn extract_request<R>(request: lsp_server::Request) -> R::Params
where
    R: lsp::request::Request,
{
    let (_, params) = request
        .extract::<R::Params>(R::METHOD)
        .expect(format!("Could not extract request {}", R::METHOD).as_str());
    params
}

fn extract_notification<N>(notification: lsp_server::Notification) -> N::Params
where
    N: lsp::notification::Notification,
{
    notification
        .extract::<N::Params>(N::METHOD)
        .expect(format!("Could not extract notification {}", N::METHOD).as_str())
}
