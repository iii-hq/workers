//! `session::read-attachment` — read a stored attachment the way an agent
//! can use it, by its id alone.
//!
//! A text file comes back as characters, one window at a time
//! (`offset` / `max_chars`, `next_offset` until the end). Any other file
//! (PDF, office document, image, anything else) comes back with
//! `how_to_read`: the exact `fp::pipe` calls that convert it to markdown or
//! save it into the working folder on this engine, so its bytes move worker
//! to worker and never through the model; a recipe whose function the engine
//! lacks names the worker to install instead. Read-only and agent-callable
//! (an explicit allow in `iii-permissions.yaml`).

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::IIIClient;
use iii_sdk::protocol::CallRequest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::Deps;
use crate::error::SessionError;
use crate::reading::{self, AttachmentKind, RecipeFunctions};
use crate::service::SessionService;
use crate::types::AttachmentMeta;

pub const FUNCTION_ID: &str = "session::read-attachment";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ReadAttachmentRequest {
    /// The attachment's id (`a_…`), from an `@attachment(id="a_…")` mention
    /// or an `<attachment id="a_…">` block.
    pub attachment_id: String,
    /// The session holding it. Optional: the id alone finds it.
    #[serde(default)]
    pub session_id: Option<String>,
    /// First character of the text window (text files only). Default 0.
    #[serde(default)]
    pub offset: Option<u64>,
    /// Characters in the text window: default 20000, at most 100000.
    #[serde(default)]
    pub max_chars: Option<u64>,
}

/// `null` when no session holds the attachment.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ReadAttachmentResponse {
    pub attachment: AttachmentMeta,
    /// From the mime type and the extension; a text file that is not UTF-8
    /// is `binary`.
    pub kind: AttachmentKind,
    /// `kind: "text"` only: the characters `[offset, offset + max_chars)`.
    pub text: Option<String>,
    /// Where `text` starts (0 for the other kinds).
    pub offset: u64,
    /// The `offset` of the next window; `null` at the end of the text and for
    /// the other kinds.
    pub next_offset: Option<u64>,
    /// Characters in the whole text; `null` for the other kinds.
    pub total_chars: Option<u64>,
    /// Text remains after this window.
    pub truncated: bool,
    /// Every kind but text: the exact calls that read or save the file on
    /// this engine (or the worker to install).
    pub how_to_read: Option<String>,
}

/// Whether a function is registered on the engine right now (the recipes
/// are only offered for functions that exist).
#[async_trait]
pub trait FunctionProbe: Send + Sync {
    async fn registered(&self, function_id: &str) -> bool;
}

/// [`FunctionProbe`] over `engine::functions::info`.
pub struct EngineProbe {
    iii: Arc<IIIClient>,
}

impl EngineProbe {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl FunctionProbe for EngineProbe {
    async fn registered(&self, function_id: &str) -> bool {
        let info = self
            .iii
            .call(CallRequest {
                function_id: "engine::functions::info".to_string(),
                payload: json!({ "function_id": function_id }),
                action: None,
                timeout_ms: Some(3_000),
            })
            .await;
        matches!(info, Ok(value) if !value.is_null())
    }
}

/// The recipes' functions, probed concurrently.
async fn recipe_functions(probe: &dyn FunctionProbe) -> RecipeFunctions {
    let (fp_pipe, pdf_to_markdown, document_to_markdown, shell_exec) = tokio::join!(
        probe.registered(reading::FP_PIPE),
        probe.registered(reading::PDF_TO_MARKDOWN),
        probe.registered(reading::DOCUMENT_TO_MARKDOWN),
        probe.registered(reading::SHELL_EXEC),
    );
    RecipeFunctions {
        fp_pipe,
        pdf_to_markdown,
        document_to_markdown,
        shell_exec,
    }
}

pub async fn handle(
    deps: &Deps,
    probe: &dyn FunctionProbe,
    req: ReadAttachmentRequest,
) -> Result<Option<ReadAttachmentResponse>, SessionError> {
    read(&deps.service, probe, req).await
}

/// The handler body, engine-free for tests.
pub async fn read(
    service: &SessionService,
    probe: &dyn FunctionProbe,
    req: ReadAttachmentRequest,
) -> Result<Option<ReadAttachmentResponse>, SessionError> {
    let hint = req.session_id.as_deref();
    let Some(meta) = service
        .find_attachment_meta(&req.attachment_id, hint)
        .await?
    else {
        return Ok(None);
    };
    if AttachmentKind::of(&meta.name, &meta.mime) == AttachmentKind::Text {
        // Only text needs the bytes here (and only text is checked for UTF-8).
        let Some((meta, bytes)) = service
            .find_attachment(&req.attachment_id, Some(&meta.session_id))
            .await?
        else {
            return Ok(None);
        };
        if let Ok(text) = std::str::from_utf8(&bytes) {
            let window = reading::text_window(text, req.offset, req.max_chars);
            return Ok(Some(ReadAttachmentResponse {
                attachment: meta,
                kind: AttachmentKind::Text,
                text: Some(window.text),
                offset: window.offset,
                next_offset: window.next_offset,
                total_chars: Some(window.total_chars),
                truncated: window.truncated,
                how_to_read: None,
            }));
        }
        return Ok(Some(recipe(AttachmentKind::Binary, meta, probe).await));
    }
    let kind = AttachmentKind::of(&meta.name, &meta.mime);
    Ok(Some(recipe(kind, meta, probe).await))
}

async fn recipe(
    kind: AttachmentKind,
    meta: AttachmentMeta,
    probe: &dyn FunctionProbe,
) -> ReadAttachmentResponse {
    let functions = recipe_functions(probe).await;
    let how_to_read = reading::how_to_read(kind, &meta, &functions);
    ReadAttachmentResponse {
        attachment: meta,
        kind,
        text: None,
        offset: 0,
        next_offset: None,
        total_chars: None,
        truncated: false,
        how_to_read,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkerConfig;
    use crate::functions::create::CreateRequest;
    use crate::functions::put_attachment::PutAttachmentRequest;
    use crate::store::FsStore;
    use base64::Engine as _;

    struct Probe(&'static [&'static str]);

    #[async_trait]
    impl FunctionProbe for Probe {
        async fn registered(&self, function_id: &str) -> bool {
            self.0.contains(&function_id)
        }
    }

    const EVERYTHING: Probe = Probe(&[
        reading::FP_PIPE,
        reading::PDF_TO_MARKDOWN,
        reading::DOCUMENT_TO_MARKDOWN,
        reading::SHELL_EXEC,
    ]);

    async fn service_with_session(dir: &tempfile::TempDir) -> (SessionService, String) {
        let store = Arc::new(FsStore::new(dir.path()).unwrap());
        let service = SessionService::new(store, &WorkerConfig::default());
        let (created, _) = service
            .create(serde_json::from_value::<CreateRequest>(json!({})).unwrap())
            .await
            .unwrap();
        (service, created.session_id)
    }

    async fn put(
        service: &SessionService,
        session_id: &str,
        name: &str,
        mime: &str,
        bytes: &[u8],
    ) -> String {
        let req: PutAttachmentRequest = serde_json::from_value(json!({
            "session_id": session_id,
            "name": name,
            "mime": mime,
            "data": base64::engine::general_purpose::STANDARD.encode(bytes),
        }))
        .unwrap();
        service
            .put_attachment(req)
            .await
            .unwrap()
            .attachment
            .attachment_id
    }

    fn request(
        attachment_id: &str,
        offset: Option<u64>,
        max_chars: Option<u64>,
    ) -> ReadAttachmentRequest {
        ReadAttachmentRequest {
            attachment_id: attachment_id.to_string(),
            session_id: None,
            offset,
            max_chars,
        }
    }

    #[tokio::test]
    async fn text_reads_in_windows_by_id_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (service, session_id) = service_with_session(&dir).await;
        let id = put(
            &service,
            &session_id,
            "notes.md",
            "text/markdown",
            "secret: émbar".as_bytes(),
        )
        .await;

        let first = read(&service, &EVERYTHING, request(&id, None, Some(8)))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.kind, AttachmentKind::Text);
        assert_eq!(first.attachment.session_id, session_id);
        assert_eq!(first.text.as_deref(), Some("secret: "));
        assert_eq!(
            (
                first.offset,
                first.next_offset,
                first.total_chars,
                first.truncated
            ),
            (0, Some(8), Some(13), true)
        );
        assert!(first.how_to_read.is_none());

        let rest = read(
            &service,
            &EVERYTHING,
            request(&id, first.next_offset, Some(8)),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(rest.text.as_deref(), Some("émbar"));
        assert_eq!(
            (rest.offset, rest.next_offset, rest.truncated),
            (8, None, false)
        );
    }

    #[tokio::test]
    async fn other_kinds_answer_a_recipe_and_unknown_ids_null() {
        let dir = tempfile::tempdir().unwrap();
        let (service, session_id) = service_with_session(&dir).await;
        let pdf = put(
            &service,
            &session_id,
            "report.pdf",
            "application/pdf",
            b"%PDF-1.4",
        )
        .await;
        let docx = put(&service, &session_id, "memo.docx", "", b"PK").await;
        let png = put(&service, &session_id, "shot.png", "image/png", b"\x89PNG").await;
        let zip = put(
            &service,
            &session_id,
            "bundle.zip",
            "application/zip",
            b"PK\x03\x04",
        )
        .await;
        let latin1 = put(&service, &session_id, "old.txt", "text/plain", b"caf\xe9").await;

        for (id, kind) in [
            (&pdf, AttachmentKind::Pdf),
            (&docx, AttachmentKind::Document),
            (&png, AttachmentKind::Image),
            (&zip, AttachmentKind::Binary),
            (&latin1, AttachmentKind::Binary),
        ] {
            let read = read(&service, &EVERYTHING, request(id, Some(3), None))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(read.kind, kind, "{}", read.attachment.name);
            assert!(read.text.is_none());
            assert_eq!(
                (
                    read.offset,
                    read.next_offset,
                    read.total_chars,
                    read.truncated
                ),
                (0, None, None, false)
            );
            let how = read.how_to_read.expect("a recipe");
            assert!(
                how.contains(&format!("\"attachment_id\":\"{id}\"")),
                "{how}"
            );
            assert!(
                how.contains(&format!("\"session_id\":\"{session_id}\"")),
                "{how}"
            );
        }
        let pdf_read = read(&service, &EVERYTHING, request(&pdf, None, None))
            .await
            .unwrap()
            .unwrap();
        assert!(pdf_read.how_to_read.unwrap().contains("pdf::to-markdown"));

        // A missing worker is named, not offered.
        let bare = read(
            &service,
            &Probe(&[reading::FP_PIPE]),
            request(&pdf, None, None),
        )
        .await
        .unwrap()
        .unwrap();
        let how = bare.how_to_read.unwrap();
        assert!(how.contains("install the pdf worker"), "{how}");
        assert!(how.contains("install the ide worker"), "{how}");

        assert!(
            read(&service, &EVERYTHING, request("a_unknown", None, None))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_session_hint_picks_the_holder_and_falls_back_to_the_id() {
        let dir = tempfile::tempdir().unwrap();
        let (service, session_id) = service_with_session(&dir).await;
        let id = put(&service, &session_id, "n.txt", "text/plain", b"hi").await;
        let mut req = request(&id, None, None);
        req.session_id = Some("s_other".into());
        let read = read(&service, &EVERYTHING, req).await.unwrap().unwrap();
        assert_eq!(read.attachment.session_id, session_id);
        assert_eq!(read.text.as_deref(), Some("hi"));
    }
}
