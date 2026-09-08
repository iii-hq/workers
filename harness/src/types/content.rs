//! Content blocks — the atomic units of message content
//! (README § Content blocks). Wire-identical to `llm-router` and
//! `session-manager` so messages round-trip across the bus untouched.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        mime: String,
        data: String,
        /// Optional link to the stored original of this inline image
        /// (`session::put-attachment`, resolvable with `session::get-attachment`).
        /// Lets readers ask `session::messages { include_image_data: false }`
        /// and fetch the bytes lazily instead of receiving them inline.
        /// Persisted verbatim on the transcript; model-bound consumers never
        /// see it (see [`ContentBlock::strip_files`]). Omitted on the wire when
        /// absent so legacy transcripts stay byte-identical.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attachment_id: Option<String>,
    },
    /// Reference to an original upload in the session-manager attachment
    /// store (resolvable with `session::get-attachment`). A REFERENCE ONLY:
    /// the bytes are never inline. Persisted verbatim on user messages and
    /// stripped from every model-bound copy (see [`ContentBlock::strip_files`]).
    File {
        attachment_id: String,
        name: String,
        mime: String,
        size: u64,
    },
    Thinking {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    /// Opaque redacted thinking payload — replayed verbatim on the wire.
    RedactedThinking {
        data: String,
    },
    FunctionCall {
        id: String,
        function_id: String,
        arguments: serde_json::Value,
    },
    FunctionResult {
        function_call_id: String,
        content: Vec<ContentBlock>,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

/// Placeholder text a model-bound message falls back to when stripping its
/// `file` references would otherwise leave it with no content at all.
pub const ATTACHMENT_PLACEHOLDER: &str = "(attachment)";

impl ContentBlock {
    /// A single text block (the common case for user/system content).
    pub fn text(s: impl Into<String>) -> Self {
        ContentBlock::Text { text: s.into() }
    }

    /// Concatenated text of all `Text` blocks in a slice, newline-joined.
    pub fn join_text(blocks: &[ContentBlock]) -> String {
        let mut out = String::new();
        for b in blocks {
            if let ContentBlock::Text { text } = b {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
        out
    }

    /// Whether `blocks` carries a `File` reference anywhere, including nested
    /// inside a `FunctionResult`.
    pub fn contains_file(blocks: &[ContentBlock]) -> bool {
        blocks.iter().any(|b| match b {
            ContentBlock::File { .. } => true,
            ContentBlock::FunctionResult { content, .. } => Self::contains_file(content),
            _ => false,
        })
    }

    /// Whether `blocks` carries any attachment reference a model-bound copy
    /// must lose — a `File` block or an `Image` linked to its stored original
    /// — including nested inside a `FunctionResult`. The cheap pre-check that
    /// lets [`crate::types::message::AgentMessage::strip_file_blocks`] skip
    /// the clone for the common reference-free message.
    pub fn contains_attachment_refs(blocks: &[ContentBlock]) -> bool {
        blocks.iter().any(|b| match b {
            ContentBlock::File { .. } => true,
            ContentBlock::Image { attachment_id, .. } => attachment_id.is_some(),
            ContentBlock::FunctionResult { content, .. } => Self::contains_attachment_refs(content),
            _ => false,
        })
    }

    /// The model-bound copy of `blocks`: every attachment reference removed —
    /// `File` blocks dropped, `Image` blocks kept (mime + data) but unlinked
    /// from their stored original (`attachment_id: None`) — recursing into
    /// `FunctionResult.content`. The persisted transcript keeps its
    /// references; only the copy handed to context assembly and the router
    /// is stripped. A non-empty input never strips to nothing — an
    /// all-file list becomes a single [`ATTACHMENT_PLACEHOLDER`] text block so
    /// providers never see an empty content array.
    pub fn strip_files(blocks: &[ContentBlock]) -> Vec<ContentBlock> {
        let mut out = Vec::with_capacity(blocks.len());
        for b in blocks {
            match b {
                ContentBlock::File { .. } => {}
                // The link is transcript bookkeeping for lazy readers; the
                // model only needs the inline bytes.
                ContentBlock::Image { mime, data, .. } => out.push(ContentBlock::Image {
                    mime: mime.clone(),
                    data: data.clone(),
                    attachment_id: None,
                }),
                ContentBlock::FunctionResult {
                    function_call_id,
                    content,
                    is_error,
                } => out.push(ContentBlock::FunctionResult {
                    function_call_id: function_call_id.clone(),
                    content: Self::strip_files(content),
                    is_error: *is_error,
                }),
                other => out.push(other.clone()),
            }
        }
        if out.is_empty() && !blocks.is_empty() {
            out.push(ContentBlock::text(ATTACHMENT_PLACEHOLDER));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn file_block() -> ContentBlock {
        ContentBlock::File {
            attachment_id: "a_3f2e".into(),
            name: "report.pdf".into(),
            mime: "application/pdf".into(),
            size: 12345,
        }
    }

    #[test]
    fn file_block_round_trips_through_serde_with_the_wire_shape() {
        let wire = json!({
            "type": "file",
            "attachment_id": "a_3f2e",
            "name": "report.pdf",
            "mime": "application/pdf",
            "size": 12345
        });
        let parsed: ContentBlock = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(parsed, file_block());
        assert_eq!(serde_json::to_value(&parsed).unwrap(), wire);
    }

    #[test]
    fn join_text_ignores_file_blocks() {
        let blocks = vec![
            ContentBlock::text("hello"),
            file_block(),
            ContentBlock::text("world"),
        ];
        assert_eq!(ContentBlock::join_text(&blocks), "hello\nworld");
        assert_eq!(ContentBlock::join_text(&[file_block()]), "");
    }

    #[test]
    fn image_attachment_id_is_optional_and_omitted_on_the_wire_when_absent() {
        // A legacy image (no link) deserializes to `None` and re-serializes
        // without the key, so existing transcripts stay byte-identical.
        let legacy = json!({ "type": "image", "mime": "image/png", "data": "AAAA" });
        let parsed: ContentBlock = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(
            parsed,
            ContentBlock::Image {
                mime: "image/png".into(),
                data: "AAAA".into(),
                attachment_id: None,
            }
        );
        assert_eq!(serde_json::to_value(&parsed).unwrap(), legacy);

        // A linked image keeps its reference through the round trip.
        let linked = json!({
            "type": "image",
            "mime": "image/png",
            "data": "AAAA",
            "attachment_id": "a_9c1d"
        });
        let parsed: ContentBlock = serde_json::from_value(linked.clone()).unwrap();
        assert_eq!(
            parsed,
            ContentBlock::Image {
                mime: "image/png".into(),
                data: "AAAA".into(),
                attachment_id: Some("a_9c1d".into()),
            }
        );
        assert_eq!(serde_json::to_value(&parsed).unwrap(), linked);
    }

    #[test]
    fn strip_files_keeps_text_and_image_and_recurses_into_results() {
        let image = ContentBlock::Image {
            mime: "image/png".into(),
            data: "AAAA".into(),
            attachment_id: Some("a_9c1d".into()),
        };
        // The model-bound copy keeps the inline bytes but loses the link.
        let unlinked_image = ContentBlock::Image {
            mime: "image/png".into(),
            data: "AAAA".into(),
            attachment_id: None,
        };
        let blocks = vec![
            ContentBlock::text("see attached"),
            file_block(),
            image.clone(),
            ContentBlock::FunctionResult {
                function_call_id: "c1".into(),
                content: vec![file_block(), ContentBlock::text("ok")],
                is_error: None,
            },
        ];
        assert!(ContentBlock::contains_file(&blocks));
        assert!(ContentBlock::contains_attachment_refs(&blocks));
        let stripped = ContentBlock::strip_files(&blocks);
        assert_eq!(
            stripped,
            vec![
                ContentBlock::text("see attached"),
                unlinked_image,
                ContentBlock::FunctionResult {
                    function_call_id: "c1".into(),
                    content: vec![ContentBlock::text("ok")],
                    is_error: None,
                },
            ]
        );
        assert!(!ContentBlock::contains_file(&stripped));
        assert!(!ContentBlock::contains_attachment_refs(&stripped));
        // The original is untouched.
        assert_eq!(blocks.len(), 4);
        assert!(ContentBlock::contains_file(&blocks));
        assert_eq!(blocks[2], image);
    }

    #[test]
    fn linked_image_alone_counts_as_an_attachment_reference_but_not_a_file() {
        let linked = ContentBlock::Image {
            mime: "image/jpeg".into(),
            data: "/9j/".into(),
            attachment_id: Some("a_77".into()),
        };
        let blocks = vec![ContentBlock::text("look"), linked];
        assert!(!ContentBlock::contains_file(&blocks));
        assert!(ContentBlock::contains_attachment_refs(&blocks));
        let stripped = ContentBlock::strip_files(&blocks);
        assert_eq!(stripped.len(), 2);
        assert_eq!(
            stripped[1],
            ContentBlock::Image {
                mime: "image/jpeg".into(),
                data: "/9j/".into(),
                attachment_id: None,
            }
        );
        // Nested inside a function result too.
        let nested = vec![ContentBlock::FunctionResult {
            function_call_id: "c1".into(),
            content: blocks.clone(),
            is_error: None,
        }];
        assert!(ContentBlock::contains_attachment_refs(&nested));
        assert!(!ContentBlock::contains_attachment_refs(
            &ContentBlock::strip_files(&nested)
        ));
    }

    #[test]
    fn strip_files_never_yields_an_empty_content_array() {
        assert_eq!(
            ContentBlock::strip_files(&[file_block()]),
            vec![ContentBlock::text(ATTACHMENT_PLACEHOLDER)]
        );
        // An already-empty list stays empty (e.g. the streaming assistant placeholder).
        assert!(ContentBlock::strip_files(&[]).is_empty());
    }
}
