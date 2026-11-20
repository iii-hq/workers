//! Content blocks — the atomic units of message content
//! (README § Content blocks). Wire-identical to `llm-router` and
//! `session-manager` so messages round-trip across the bus untouched.

use std::collections::HashSet;

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
    /// the bytes are never inline. Persisted verbatim on user messages; every
    /// model-bound copy replaces it with an `<attachment>` text block naming
    /// the call that reads it (see [`ContentBlock::strip_files`]).
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

/// The function the `<attachment>` block tells the model to call.
pub const READ_ATTACHMENT_FUNCTION: &str = "session::read-attachment";

/// `value` escaped for an XML attribute: `& < > " '` and every control
/// character become character references, so a file name can never close
/// the attribute or the tag.
pub fn escape_xml_attribute(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() => out.push_str(&format!("&#{};", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// The text that stands in for a `file` block in the model-bound copy:
///
/// ```text
/// <attachment id="a_…" name="report.pdf" mime="application/pdf" size="1234567">
/// Read it with session::read-attachment {"session_id":"<sid>","attachment_id":"a_…"}
/// </attachment>
/// ```
///
/// Attribute values are escaped as XML attributes; the body is one line that
/// names the exact call (without `session_id` when the session is unknown:
/// the id alone finds the file). When the message also carries the file's
/// picture (`image_twin`), the body says so before the call.
pub fn attachment_text(
    attachment_id: &str,
    name: &str,
    mime: &str,
    size: u64,
    session_id: Option<&str>,
    image_twin: bool,
) -> String {
    let quoted = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string());
    let args = match session_id {
        Some(sid) => format!(
            "{{\"session_id\":{},\"attachment_id\":{}}}",
            quoted(sid),
            quoted(attachment_id)
        ),
        None => format!("{{\"attachment_id\":{}}}", quoted(attachment_id)),
    };
    let body = if image_twin {
        format!(
            "The picture is in this message; read the file with {READ_ATTACHMENT_FUNCTION} {args}"
        )
    } else {
        format!("Read it with {READ_ATTACHMENT_FUNCTION} {args}")
    };
    format!(
        "<attachment id=\"{}\" name=\"{}\" mime=\"{}\" size=\"{size}\">\n{body}\n</attachment>",
        escape_xml_attribute(attachment_id),
        escape_xml_attribute(name),
        escape_xml_attribute(mime),
    )
}

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
    /// must rewrite — a `File` block or an `Image` linked to its stored
    /// original — including nested inside a `FunctionResult`. The cheap
    /// pre-check that lets
    /// [`crate::types::message::AgentMessage::strip_file_blocks`] skip the
    /// clone for the common reference-free message.
    pub fn contains_attachment_refs(blocks: &[ContentBlock]) -> bool {
        blocks.iter().any(|b| match b {
            ContentBlock::File { .. } => true,
            ContentBlock::Image { attachment_id, .. } => attachment_id.is_some(),
            ContentBlock::FunctionResult { content, .. } => Self::contains_attachment_refs(content),
            _ => false,
        })
    }

    /// The `attachment_id` of every linked `Image` in `blocks` (recursing into
    /// `FunctionResult`): the files whose picture the message already carries.
    fn image_twins(blocks: &[ContentBlock]) -> HashSet<String> {
        let mut twins = HashSet::new();
        for b in blocks {
            match b {
                ContentBlock::Image {
                    attachment_id: Some(id),
                    ..
                } => {
                    twins.insert(id.clone());
                }
                ContentBlock::FunctionResult { content, .. } => {
                    twins.extend(Self::image_twins(content))
                }
                _ => {}
            }
        }
        twins
    }

    /// The model-bound copy of one message's `blocks`: every `File` block
    /// replaced, in place, by its `<attachment>` text block
    /// ([`attachment_text`], naming `session::read-attachment` for
    /// `session_id`), and every `Image` kept (mime + data) but unlinked from
    /// its stored original (`attachment_id: None`) — recursing into
    /// `FunctionResult.content`. A file whose picture the same message
    /// carries says so. The persisted transcript keeps its references; only
    /// the copy handed to context assembly and the router is rewritten.
    /// Nothing is dropped, so a non-empty list never becomes empty.
    pub fn strip_files(blocks: &[ContentBlock], session_id: Option<&str>) -> Vec<ContentBlock> {
        Self::rewrite_files(blocks, session_id, &Self::image_twins(blocks))
    }

    fn rewrite_files(
        blocks: &[ContentBlock],
        session_id: Option<&str>,
        twins: &HashSet<String>,
    ) -> Vec<ContentBlock> {
        blocks
            .iter()
            .map(|b| match b {
                ContentBlock::File {
                    attachment_id,
                    name,
                    mime,
                    size,
                } => ContentBlock::text(attachment_text(
                    attachment_id,
                    name,
                    mime,
                    *size,
                    session_id,
                    twins.contains(attachment_id),
                )),
                // The link is transcript bookkeeping for lazy readers; the
                // model only needs the inline bytes.
                ContentBlock::Image { mime, data, .. } => ContentBlock::Image {
                    mime: mime.clone(),
                    data: data.clone(),
                    attachment_id: None,
                },
                ContentBlock::FunctionResult {
                    function_call_id,
                    content,
                    is_error,
                } => ContentBlock::FunctionResult {
                    function_call_id: function_call_id.clone(),
                    content: Self::rewrite_files(content, session_id, twins),
                    is_error: *is_error,
                },
                other => other.clone(),
            })
            .collect()
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

    const REPORT_BLOCK: &str = "<attachment id=\"a_3f2e\" name=\"report.pdf\" mime=\"application/pdf\" size=\"12345\">\nRead it with session::read-attachment {\"session_id\":\"s_1\",\"attachment_id\":\"a_3f2e\"}\n</attachment>";

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
    fn the_attachment_block_names_the_read_call() {
        assert_eq!(
            attachment_text(
                "a_3f2e",
                "report.pdf",
                "application/pdf",
                12345,
                Some("s_1"),
                false
            ),
            REPORT_BLOCK
        );
        // Without a session the id alone is the call.
        assert_eq!(
            attachment_text("a_1", "n.md", "text/markdown", 3, None, false),
            "<attachment id=\"a_1\" name=\"n.md\" mime=\"text/markdown\" size=\"3\">\nRead it with session::read-attachment {\"attachment_id\":\"a_1\"}\n</attachment>"
        );
    }

    #[test]
    fn attribute_values_are_escaped_and_the_call_stays_valid_json() {
        let text = attachment_text(
            "a_1",
            "q\"&<x>'\n.txt",
            "text/plain",
            1,
            Some("s \"odd\""),
            false,
        );
        let first = text.lines().next().unwrap();
        assert_eq!(
            first,
            "<attachment id=\"a_1\" name=\"q&quot;&amp;&lt;x&gt;&apos;&#10;.txt\" mime=\"text/plain\" size=\"1\">"
        );
        assert_eq!(text.lines().count(), 3);
        let call = text.lines().nth(1).unwrap();
        let args = call
            .strip_prefix("Read it with session::read-attachment ")
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(args).unwrap();
        assert_eq!(
            parsed,
            json!({ "session_id": "s \"odd\"", "attachment_id": "a_1" })
        );
    }

    #[test]
    fn strip_files_replaces_files_in_place_unlinks_images_and_recurses() {
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
        let stripped = ContentBlock::strip_files(&blocks, Some("s_1"));
        assert_eq!(
            stripped,
            vec![
                ContentBlock::text("see attached"),
                ContentBlock::text(REPORT_BLOCK),
                unlinked_image,
                ContentBlock::FunctionResult {
                    function_call_id: "c1".into(),
                    content: vec![ContentBlock::text(REPORT_BLOCK), ContentBlock::text("ok")],
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
    fn a_file_whose_picture_is_in_the_message_says_so() {
        let photo = ContentBlock::File {
            attachment_id: "a_77".into(),
            name: "photo.jpg".into(),
            mime: "image/jpeg".into(),
            size: 2048,
        };
        let twin = ContentBlock::Image {
            mime: "image/jpeg".into(),
            data: "/9j/".into(),
            attachment_id: Some("a_77".into()),
        };
        let stripped = ContentBlock::strip_files(
            &[ContentBlock::text("look"), photo, file_block(), twin],
            Some("s_1"),
        );
        assert_eq!(
            stripped[1],
            ContentBlock::text(
                "<attachment id=\"a_77\" name=\"photo.jpg\" mime=\"image/jpeg\" size=\"2048\">\nThe picture is in this message; read the file with session::read-attachment {\"session_id\":\"s_1\",\"attachment_id\":\"a_77\"}\n</attachment>"
            )
        );
        // A file without a picture in the message keeps the plain body.
        assert_eq!(stripped[2], ContentBlock::text(REPORT_BLOCK));
        assert_eq!(
            stripped[3],
            ContentBlock::Image {
                mime: "image/jpeg".into(),
                data: "/9j/".into(),
                attachment_id: None,
            }
        );
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
        let stripped = ContentBlock::strip_files(&blocks, None);
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
            &ContentBlock::strip_files(&nested, None)
        ));
    }

    #[test]
    fn a_message_of_files_only_becomes_their_attachment_blocks() {
        assert_eq!(
            ContentBlock::strip_files(&[file_block()], Some("s_1")),
            vec![ContentBlock::text(REPORT_BLOCK)]
        );
        // An already-empty list stays empty (e.g. the streaming assistant placeholder).
        assert!(ContentBlock::strip_files(&[], Some("s_1")).is_empty());
    }
}
