//! `session::read-attachment` — read a stored attachment the way an agent
//! can use it, by its id alone.
//!
//! A text file comes back as characters, one window at a time
//! (`offset` / `max_chars`, `next_offset` until the end), and so does an
//! office document, as the markdown `document::to-markdown` makes of it. Any
//! other file (PDF, image, anything else) comes back with `how_to_read`: the
//! exact `fp::pipe` calls that convert it to markdown or save a copy in a
//! temporary folder outside the working folder on this engine, so its bytes
//! move worker to worker and never through the model; a recipe whose function
//! the engine lacks names the worker to install instead. Read-only and
//! agent-callable (an explicit allow in `iii-permissions.yaml`).

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use iii_sdk::IIIClient;
use iii_sdk::protocol::CallRequest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::Deps;
use crate::error::SessionError;
use crate::reading::{self, AttachmentKind, RecipeFunctions};
use crate::service::SessionService;
use crate::types::AttachmentMeta;

pub const FUNCTION_ID: &str = "session::read-attachment";

const CONVERT_TIMEOUT_MS: u64 = 60_000;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ReadAttachmentRequest {
    /// The attachment's id (`a_…`), from an `@attachment(id="a_…")` mention
    /// or an `<attachment id="a_…">` block.
    pub attachment_id: String,
    /// The session holding it. Optional: the id alone finds it.
    #[serde(default)]
    pub session_id: Option<String>,
    /// First character of the text window (text files and office documents
    /// only). Default 0.
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
    /// The characters `[offset, offset + max_chars)` of a text file or of an
    /// office document's markdown; `null` for the other kinds and for a
    /// document without text or that could not be converted.
    pub text: Option<String>,
    /// Where `text` starts (0 without `text`).
    pub offset: u64,
    /// The `offset` of the next window; `null` at the end of the text and
    /// without `text`.
    pub next_offset: Option<u64>,
    /// Characters in the whole text; `null` without `text`.
    pub total_chars: Option<u64>,
    /// Text remains after this window.
    pub truncated: bool,
    /// Without `text`: the exact calls that read or save the file on this
    /// engine (or the worker to install) and, for a document, why it has no
    /// text. With a document's text: the embedded images it leaves out, if any.
    pub how_to_read: Option<String>,
}

/// The calls a read makes to other workers, so tests need no engine.
#[async_trait]
pub trait Bus: Send + Sync {
    /// Whether a function is registered on the engine right now (the recipes
    /// are only offered for functions that exist).
    async fn registered(&self, function_id: &str) -> bool;

    /// The function's answer, or why there is none.
    async fn call(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: u64,
    ) -> Result<Value, String>;
}

/// [`Bus`] over the worker's client.
pub struct EngineBus {
    iii: Arc<IIIClient>,
}

impl EngineBus {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl Bus for EngineBus {
    async fn registered(&self, function_id: &str) -> bool {
        let payload = json!({ "function_id": function_id });
        let info = self.call("engine::functions::info", payload, 3_000).await;
        matches!(info, Ok(value) if !value.is_null())
    }

    async fn call(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: u64,
    ) -> Result<Value, String> {
        self.iii
            .call(CallRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms: Some(timeout_ms),
            })
            .await
            .map_err(|e| e.to_string())
    }
}

/// The recipes' functions, probed concurrently.
async fn recipe_functions(bus: &dyn Bus) -> RecipeFunctions {
    let (fp_pipe, pdf_to_markdown, shell_exec) = tokio::join!(
        bus.registered(reading::FP_PIPE),
        bus.registered(reading::PDF_TO_MARKDOWN),
        bus.registered(reading::SHELL_EXEC),
    );
    RecipeFunctions {
        fp_pipe,
        pdf_to_markdown,
        shell_exec,
    }
}

pub async fn handle(
    deps: &Deps,
    bus: &dyn Bus,
    req: ReadAttachmentRequest,
) -> Result<Option<ReadAttachmentResponse>, SessionError> {
    read(&deps.service, bus, req).await
}

/// The handler body, engine-free for tests.
pub async fn read(
    service: &SessionService,
    bus: &dyn Bus,
    req: ReadAttachmentRequest,
) -> Result<Option<ReadAttachmentResponse>, SessionError> {
    let hint = req.session_id.as_deref();
    let Some(meta) = service
        .find_attachment_meta(&req.attachment_id, hint)
        .await?
    else {
        return Ok(None);
    };
    let kind = AttachmentKind::of(&meta.name, &meta.mime);
    if matches!(kind, AttachmentKind::Text | AttachmentKind::Document) {
        // Only these need the bytes here (and only text is checked for UTF-8).
        let Some((meta, bytes)) = service
            .find_attachment(&req.attachment_id, Some(&meta.session_id))
            .await?
        else {
            return Ok(None);
        };
        if kind == AttachmentKind::Document {
            return Ok(Some(document(meta, bytes, bus, &req).await));
        }
        if let Ok(text) = std::str::from_utf8(&bytes) {
            return Ok(Some(window(kind, meta, text, &req, None)));
        }
        return Ok(Some(recipe(AttachmentKind::Binary, meta, bus).await));
    }
    Ok(Some(recipe(kind, meta, bus).await))
}

/// An office document as the markdown `document::to-markdown` makes of it.
async fn document(
    meta: AttachmentMeta,
    bytes: Vec<u8>,
    bus: &dyn Bus,
    req: &ReadAttachmentRequest,
) -> ReadAttachmentResponse {
    let kind = AttachmentKind::Document;
    let payload = json!({
        "bytes_base64": BASE64.encode(bytes),
        "file_name": meta.name,
        "max_chars": 0,
    });
    let answer = bus
        .call(reading::DOCUMENT_TO_MARKDOWN, payload, CONVERT_TIMEOUT_MS)
        .await;
    let reason = match &answer {
        Ok(answer) => match answer.pointer("/body/text").and_then(Value::as_str) {
            Some(text) => {
                let images = answer["asset_count"].as_u64().unwrap_or_default();
                let note = reading::document_note(&meta, text, images);
                if text.trim().is_empty() {
                    let functions = recipe_functions(bus).await;
                    let save = reading::save_sentence(&meta, reading::SAVE_LEAD, &functions);
                    return textless(kind, meta, note.map(|note| format!("{note} {save}")));
                }
                return window(kind, meta, text, req, note);
            }
            None => "its answer has no body.text",
        },
        Err(reason) => reason,
    };
    let (functions, present) = tokio::join!(
        recipe_functions(bus),
        bus.registered(reading::DOCUMENT_TO_MARKDOWN),
    );
    let how_to_read = reading::unconverted(&meta, present.then_some(reason), &functions);
    textless(kind, meta, Some(how_to_read))
}

async fn recipe(
    kind: AttachmentKind,
    meta: AttachmentMeta,
    bus: &dyn Bus,
) -> ReadAttachmentResponse {
    let functions = recipe_functions(bus).await;
    let how_to_read = reading::how_to_read(kind, &meta, &functions);
    textless(kind, meta, how_to_read)
}

fn window(
    kind: AttachmentKind,
    meta: AttachmentMeta,
    text: &str,
    req: &ReadAttachmentRequest,
    how_to_read: Option<String>,
) -> ReadAttachmentResponse {
    let window = reading::text_window(text, req.offset, req.max_chars);
    ReadAttachmentResponse {
        attachment: meta,
        kind,
        text: Some(window.text),
        offset: window.offset,
        next_offset: window.next_offset,
        total_chars: Some(window.total_chars),
        truncated: window.truncated,
        how_to_read,
    }
}

fn textless(
    kind: AttachmentKind,
    meta: AttachmentMeta,
    how_to_read: Option<String>,
) -> ReadAttachmentResponse {
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

    /// The engine a test sees: the functions it has (`None`: a read that must
    /// not ask) and what `document::to-markdown` answers.
    struct Stub {
        functions: Option<&'static [&'static str]>,
        convert: fn(Value) -> Result<Value, String>,
    }

    #[async_trait]
    impl Bus for Stub {
        async fn registered(&self, function_id: &str) -> bool {
            let functions = self.functions.expect("this read must not probe");
            functions.contains(&function_id)
        }

        async fn call(
            &self,
            function_id: &str,
            payload: Value,
            timeout_ms: u64,
        ) -> Result<Value, String> {
            assert_eq!(
                (function_id, timeout_ms),
                (reading::DOCUMENT_TO_MARKDOWN, 60_000)
            );
            (self.convert)(payload)
        }
    }

    const EVERYTHING: Stub = Stub {
        functions: Some(&[
            reading::FP_PIPE,
            reading::PDF_TO_MARKDOWN,
            reading::DOCUMENT_TO_MARKDOWN,
            reading::SHELL_EXEC,
        ]),
        convert: |_| panic!("only an office document is converted"),
    };

    /// A `memo.docx` read through `bus`.
    async fn read_memo(
        bus: &Stub,
        bytes: &[u8],
        max_chars: Option<u64>,
    ) -> Vec<ReadAttachmentResponse> {
        let dir = tempfile::tempdir().unwrap();
        let (service, session_id) = service_with_session(&dir).await;
        let id = put(&service, &session_id, "memo.docx", "", bytes).await;
        let mut windows: Vec<ReadAttachmentResponse> = Vec::new();
        loop {
            let offset = windows.last().and_then(|last| last.next_offset);
            let window = read(&service, bus, request(&id, offset, max_chars))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(window.kind, AttachmentKind::Document);
            assert_eq!(window.attachment.attachment_id, id);
            let end = window.next_offset.is_none();
            windows.push(window);
            if end {
                return windows;
            }
        }
    }

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
            "data": BASE64.encode(bytes),
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
            &Stub {
                functions: Some(&[reading::FP_PIPE, reading::DOCUMENT_TO_MARKDOWN]),
                ..EVERYTHING
            },
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

    #[tokio::test]
    async fn an_office_document_reads_as_text_in_windows() {
        // Its first six bytes hold `/` and `+` in base64: the URL-safe alphabet fails.
        let text = "???>>># Higienização\n\n| Superfície | Produto |\n|---|---|\n| Pia | Cloro |\n";
        // A converter whose markdown is the file itself.
        let bus = Stub {
            functions: None,
            convert: |payload| {
                assert_eq!(payload["file_name"], "memo.docx");
                assert_eq!(payload["max_chars"], 0);
                let bytes = BASE64
                    .decode(payload["bytes_base64"].as_str().unwrap())
                    .unwrap();
                let text = String::from_utf8(bytes).unwrap();
                Ok(json!({ "body": { "text": text }, "asset_count": 0 }))
            },
        };

        let windows = read_memo(&bus, text.as_bytes(), Some(7)).await;
        let total = text.chars().count() as u64;
        assert_eq!(windows.len() as u64, total.div_ceil(7));
        let joined: String = windows.iter().filter_map(|w| w.text.as_deref()).collect();
        assert_eq!(joined, text);
        for (i, window) in windows.iter().enumerate() {
            assert_eq!(window.offset, i as u64 * 7);
            assert_eq!(window.total_chars, Some(total));
            assert_eq!(window.truncated, window.next_offset.is_some());
            assert_eq!(window.how_to_read, None);
        }
    }

    #[tokio::test]
    async fn a_documents_embedded_images_are_named() {
        let one: fn(Value) -> _ = |_| Ok(json!({ "body": { "text": "ab" }, "asset_count": 1 }));
        let three: fn(Value) -> _ = |_| Ok(json!({ "body": { "text": "ab" }, "asset_count": 3 }));
        for (convert, note) in [
            (one, "1 embedded image is not in this text."),
            (three, "3 embedded images are not in this text."),
        ] {
            let bus = Stub {
                functions: None,
                convert,
            };
            let windows = read_memo(&bus, b"PK", Some(1)).await;
            assert_eq!(windows.len(), 2);
            for window in windows {
                assert!(window.text.is_some());
                assert_eq!(window.how_to_read.as_deref(), Some(note));
            }
        }
    }

    #[tokio::test]
    async fn a_document_without_text_says_so() {
        let empty: fn(Value) -> _ = |_| Ok(json!({ "body": { "text": " \n" }, "asset_count": 0 }));
        let one: fn(Value) -> _ = |_| Ok(json!({ "body": { "text": "" }, "asset_count": 1 }));
        let two: fn(Value) -> _ = |_| Ok(json!({ "body": { "text": "" }, "asset_count": 2 }));
        for (convert, note) in [
            (empty, r#""memo.docx" holds no text."#),
            (one, r#""memo.docx" holds no text, only 1 embedded image."#),
            (two, r#""memo.docx" holds no text, only 2 embedded images."#),
        ] {
            let bus = Stub {
                convert,
                ..EVERYTHING
            };
            let windows = read_memo(&bus, b"PK", None).await;
            assert_eq!(windows.len(), 1);
            assert_eq!(windows[0].text, None);
            assert_eq!(
                (windows[0].total_chars, windows[0].truncated),
                (None, false)
            );
            let how = windows[0].how_to_read.as_deref().expect("why, and a way");
            let save = format!(
                "{note} To open it with your own tools, save a copy in a temporary folder outside your working folder: {} prints ",
                reading::save_call(&windows[0].attachment)
            );
            assert!(how.starts_with(&save), "{how}");
        }
    }

    #[tokio::test]
    async fn a_failed_conversion_gives_its_reason_and_the_save_call() {
        let failed: fn(Value) -> _ = |_| Err("the document is encrypted.".to_string());
        let shapeless: fn(Value) -> _ = |_| Ok(json!({ "body": {} }));
        for (convert, reason) in [
            (
                failed,
                "document::to-markdown could not convert it: the document is encrypted. ",
            ),
            (
                shapeless,
                "document::to-markdown could not convert it: its answer has no body.text. ",
            ),
        ] {
            let bus = Stub {
                convert,
                ..EVERYTHING
            };
            let windows = read_memo(&bus, b"PK", None).await;
            assert_eq!(windows[0].text, None);
            let how = windows[0].how_to_read.as_deref().expect("a way to read it");
            assert!(
                how.starts_with(r#""memo.docx" is an office document. "#),
                "{how}"
            );
            assert!(how.contains(reason), "{how}");
            assert!(
                how.contains(&reading::save_call(&windows[0].attachment)),
                "{how}"
            );
            assert!(!how.contains("compose::add"), "{how}");
        }
    }

    #[tokio::test]
    async fn without_the_document_worker_a_document_names_the_install_and_save_calls() {
        let bus = Stub {
            functions: Some(&[
                reading::FP_PIPE,
                reading::PDF_TO_MARKDOWN,
                reading::SHELL_EXEC,
            ]),
            convert: |_| Err("remote error (NOT_FOUND)".into()),
        };
        let windows = read_memo(&bus, b"PK", None).await;
        assert_eq!(windows[0].text, None);
        let how = windows[0].how_to_read.as_deref().expect("a way to read it");
        assert!(
            how.contains(
                r#"Reading it needs document::to-markdown, which this engine does not have: install the document worker with compose::add {"worker":"document"}, then make this same call again."#
            ),
            "{how}"
        );
        assert!(
            how.contains(&reading::save_call(&windows[0].attachment)),
            "{how}"
        );
        assert!(how.contains("outside your working folder"), "{how}");
        assert!(!how.contains("NOT_FOUND"), "{how}");

        // Reading it never needs the workers a save needs.
        let cases: [(&'static [&'static str], &str); 2] = [
            (&[], "install the fp worker."),
            (&[reading::FP_PIPE], "install the ide worker."),
        ];
        for (functions, worker) in cases {
            let bus = Stub {
                functions: Some(functions),
                ..bus
            };
            let windows = read_memo(&bus, b"PK", None).await;
            let how = windows[0].how_to_read.as_deref().expect("a way to read it");
            assert!(
                how.contains(r#"compose::add {"worker":"document"}"#),
                "{how}"
            );
            assert!(how.ends_with(worker), "{how}");
            assert!(!how.contains("fp::pipe {"), "{how}");
        }
    }
}
