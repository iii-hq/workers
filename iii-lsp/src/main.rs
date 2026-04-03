use clap::Parser as ClapParser;
use dashmap::DashMap;
use std::sync::Arc;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::*;
use tower_lsp_server::{Client, LanguageServer, LspService, Server};

mod analyzer;
mod completions;
mod diagnostics;
mod engine_client;
mod hover;

#[derive(ClapParser, Debug)]
#[command(name = "iii-lsp", about = "Language Server for the III engine")]
struct Cli {
    /// WebSocket URL of the III engine
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,

    /// Accepted for compatibility with editors that pass --stdio (always uses stdio)
    #[arg(long, hide = true, default_value_t = false)]
    stdio: bool,
}

struct Backend {
    client: Client,
    engine: Arc<engine_client::EngineClient>,
    documents: DashMap<Uri, String>,
}

impl Backend {
    async fn run_diagnostics(&self, uri: Uri, source: &str) {
        let diags = diagnostics::diagnose(source, &self.engine);
        self.client.publish_diagnostics(uri, diags, None).await;
    }
}

impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        "'".to_string(),
                        "\"".to_string(),
                        ":".to_string(),
                        "{".to_string(),
                        " ".to_string(),
                    ]),
                    resolve_provider: Some(false),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "iii-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        // Start engine connection and seed cache
        self.engine.start().await;

        if self.engine.is_connected() {
            self.client
                .log_message(
                    MessageType::INFO,
                    format!(
                        "iii-lsp: connected to engine ({} functions, {} trigger types)",
                        self.engine.functions.len(),
                        self.engine.trigger_types.len()
                    ),
                )
                .await;
        } else {
            self.client
                .log_message(
                    MessageType::WARNING,
                    "iii-lsp: engine not running, completions will be empty until engine starts",
                )
                .await;
        }
    }

    async fn shutdown(&self) -> Result<()> {
        self.engine.shutdown().await;
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;
        self.run_diagnostics(uri.clone(), &text).await;
        self.documents.insert(uri, text);
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.into_iter().last() {
            let uri = params.text_document.uri;
            self.run_diagnostics(uri.clone(), &change.text).await;
            self.documents.insert(uri, change.text);
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        // Clear diagnostics when file is closed
        self.client
            .publish_diagnostics(params.text_document.uri.clone(), Vec::new(), None)
            .await;
        self.documents.remove(&params.text_document.uri);
    }

    async fn completion(
        &self,
        params: CompletionParams,
    ) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let source = match self.documents.get(uri) {
            Some(doc) => doc.value().clone(),
            None => return Ok(None),
        };

        let result = analyzer::analyze(&source, position);
        let items = completions::get_completions(&result.context, &self.engine);

        if items.is_empty() {
            Ok(None)
        } else {
            Ok(Some(CompletionResponse::Array(items)))
        }
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let source = match self.documents.get(uri) {
            Some(doc) => doc.value().clone(),
            None => return Ok(None),
        };

        let result = analyzer::analyze(&source, position);

        if result.current_text.is_empty() {
            return Ok(None);
        }

        Ok(hover::get_hover(&result.current_text, &self.engine))
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    tracing::info!("starting iii-lsp, connecting to {}", cli.url);

    let engine = engine_client::EngineClient::new(&cli.url);

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend {
        client,
        engine: Arc::clone(&engine),
        documents: DashMap::new(),
    });

    Server::new(stdin, stdout, socket).serve(service).await;

    // Clean shutdown after server exits
    engine.shutdown().await;
}
