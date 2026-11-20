//! What an attached file is and how an agent reads it: the kind
//! (`session::read-attachment`, the `@attachment` mention), the character
//! window of a text file, the human size, and the recipes (`how_to_read`)
//! for everything that is not text.
//!
//! The kind follows the ADE's `classifyAttachment`
//! (`ade/web/src/lib/attachments/`): a PDF, then an office document, then
//! markup (SVG, HTML, XML read as the text they are), then an image, then
//! text; anything else is binary. A text file whose bytes are not UTF-8 is
//! binary too ([`AttachmentKind::of_bytes`]).
//!
//! Every recipe is one `fp::pipe` that starts from `session::get-attachment`,
//! so the bytes travel worker to worker and never through the model.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::AttachmentMeta;

/// What `session::read-attachment` makes of a stored file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentKind {
    /// UTF-8 text, returned in character windows.
    Text,
    /// A picture.
    Image,
    /// A PDF, read through `pdf::to-markdown`.
    Pdf,
    /// An office document (Word, Excel, PowerPoint, OpenDocument, RTF, EPUB,
    /// CSV), read through `document::to-markdown`.
    Document,
    /// Anything else: saved into the working folder and opened with the
    /// agent's own tools.
    Binary,
}

/// Extensions read as text whatever the declared type (the ADE's
/// `TEXT_EXTENSIONS`).
pub const TEXT_EXTENSIONS: &[&str] = &[
    "txt",
    "md",
    "markdown",
    "mdx",
    "rst",
    "log",
    "json",
    "jsonl",
    "yaml",
    "yml",
    "toml",
    "ini",
    "env",
    "xml",
    "svg",
    "html",
    "htm",
    "css",
    "scss",
    "sql",
    "sh",
    "bash",
    "zsh",
    "fish",
    "ps1",
    "js",
    "jsx",
    "mjs",
    "cjs",
    "ts",
    "tsx",
    "rs",
    "go",
    "py",
    "rb",
    "java",
    "kt",
    "swift",
    "c",
    "h",
    "cc",
    "cpp",
    "hpp",
    "cs",
    "php",
    "lua",
    "r",
    "dockerfile",
    "gitignore",
    "diff",
    "patch",
];

/// Declared types read as text besides `text/*`.
const TEXT_MIMES: &[&str] = &["application/json", "application/xml", "application/x-yaml"];

/// Extensions the `document` worker converts (the ADE's
/// `DOCUMENT_EXTENSIONS`; `pdf` has its own worker).
pub const DOCUMENT_EXTENSIONS: &[&str] = &[
    "doc", "docx", "docm", "ppt", "pps", "pot", "pptx", "pptm", "ppsx", "ppsm", "xls", "xlsx",
    "xlsm", "xlsb", "odt", "ods", "odp", "rtf", "epub", "csv",
];

/// Picture extensions (the ADE's `IMAGE_EXTENSIONS`).
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "bmp", "tif", "tiff", "avif",
];

/// Markup a browser may call an image or a document, read as text.
const MARKUP_EXTENSIONS: &[&str] = &["svg", "html", "htm", "xml"];
const MARKUP_MIMES: &[&str] = &["image/svg+xml", "text/html", "application/xml", "text/xml"];

/// Default and ceiling of a text window, in characters.
pub const DEFAULT_MAX_CHARS: u64 = 20_000;
pub const MAX_MAX_CHARS: u64 = 100_000;

/// Characters a recipe's receipt shows (`fp::pipe`'s `preview_chars` cap).
pub const PREVIEW_CHARS: u64 = 8_000;

/// The functions the recipes call, probed on the engine before a recipe is
/// offered.
pub const FP_PIPE: &str = "fp::pipe";
pub const PDF_TO_MARKDOWN: &str = "pdf::to-markdown";
pub const DOCUMENT_TO_MARKDOWN: &str = "document::to-markdown";
pub const SHELL_EXEC: &str = "shell::exec";

/// The lowercase extension after the last dot; empty for none, a leading
/// dot only (`.env`) or a trailing dot (the ADE's `extensionOf`).
pub fn extension_of(name: &str) -> String {
    match name.rfind('.') {
        Some(dot) if dot > 0 && dot + 1 < name.len() => name[dot + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// The declared type without parameters, lowercase (`text/plain; charset=utf-8`
/// → `text/plain`).
fn bare_mime(mime: &str) -> String {
    mime.split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

impl AttachmentKind {
    /// The kind by name and declared type alone: what a mention shows without
    /// reading the bytes.
    pub fn of(name: &str, mime: &str) -> Self {
        let ext = extension_of(name);
        let ext = ext.as_str();
        let mime = bare_mime(mime);
        let mime = mime.as_str();
        if mime == "application/pdf" || ext == "pdf" {
            AttachmentKind::Pdf
        } else if DOCUMENT_EXTENSIONS.contains(&ext) {
            AttachmentKind::Document
        } else if MARKUP_EXTENSIONS.contains(&ext) || MARKUP_MIMES.contains(&mime) {
            AttachmentKind::Text
        } else if mime.starts_with("image/") || IMAGE_EXTENSIONS.contains(&ext) {
            AttachmentKind::Image
        } else if TEXT_EXTENSIONS.contains(&ext)
            || mime.starts_with("text/")
            || TEXT_MIMES.contains(&mime)
        {
            AttachmentKind::Text
        } else {
            AttachmentKind::Binary
        }
    }

    /// The kind `session::read-attachment` answers: [`AttachmentKind::of`],
    /// with a text file whose bytes are not UTF-8 demoted to binary.
    pub fn of_bytes(name: &str, mime: &str, bytes: &[u8]) -> Self {
        match Self::of(name, mime) {
            AttachmentKind::Text if std::str::from_utf8(bytes).is_err() => AttachmentKind::Binary,
            kind => kind,
        }
    }

    /// The word a mention's description uses (`PDF · 1.2 MB`).
    pub fn label(self) -> &'static str {
        match self {
            AttachmentKind::Text => "Text",
            AttachmentKind::Image => "Image",
            AttachmentKind::Pdf => "PDF",
            AttachmentKind::Document => "Document",
            AttachmentKind::Binary => "File",
        }
    }
}

/// A size for people: `512 B`, `1.2 MB`, `338 KB`, `11 MB` (1024-based, one
/// decimal below 10).
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if value < 10.0 {
        let rounded = format!("{value:.1}");
        let trimmed = rounded.strip_suffix(".0").unwrap_or(&rounded);
        format!("{trimmed} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// One window of a text, in characters (Unicode scalar values).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextWindow {
    pub text: String,
    /// Where the window starts (the requested offset, at most the length).
    pub offset: u64,
    /// Where the next window starts; `None` at the end.
    pub next_offset: Option<u64>,
    pub total_chars: u64,
    /// Text remains after this window.
    pub truncated: bool,
}

/// The window `[offset, offset + max_chars)` of `text`; `max_chars` is
/// clamped to `1..=MAX_MAX_CHARS` and defaults to `DEFAULT_MAX_CHARS`.
pub fn text_window(text: &str, offset: Option<u64>, max_chars: Option<u64>) -> TextWindow {
    let max_chars = max_chars
        .unwrap_or(DEFAULT_MAX_CHARS)
        .clamp(1, MAX_MAX_CHARS);
    let total = text.chars().count() as u64;
    let start = offset.unwrap_or(0).min(total);
    let end = start.saturating_add(max_chars).min(total);
    let window: String = text
        .chars()
        .skip(start as usize)
        .take((end - start) as usize)
        .collect();
    let next_offset = (end < total).then_some(end);
    TextWindow {
        text: window,
        offset: start,
        next_offset,
        total_chars: total,
        truncated: next_offset.is_some(),
    }
}

/// Which of the recipes' functions the engine has right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecipeFunctions {
    pub fp_pipe: bool,
    pub pdf_to_markdown: bool,
    pub document_to_markdown: bool,
    pub shell_exec: bool,
}

impl RecipeFunctions {
    pub const ALL: RecipeFunctions = RecipeFunctions {
        fp_pipe: true,
        pdf_to_markdown: true,
        document_to_markdown: true,
        shell_exec: true,
    };
}

fn quoted(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

/// The name a saved copy gets in the working folder: the attachment's name
/// with path separators and control characters replaced, never empty, `.`
/// or `..`.
pub fn saved_file_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    match cleaned.as_str() {
        "" | "." | ".." => "attachment".to_string(),
        _ => cleaned,
    }
}

/// The two steps every recipe starts with: the stored bytes, then their
/// base64 alone.
fn fetch_steps(meta: &AttachmentMeta) -> String {
    format!(
        r#"{{"function":"session::get-attachment","payload":{{"session_id":{},"attachment_id":{}}}}},{{"function":"fp::get","payload":{{"path":"/data"}}}}"#,
        quoted(&meta.session_id),
        quoted(&meta.attachment_id),
    )
}

/// `fp::pipe` that converts the file with `function` (`pdf::to-markdown` or
/// `document::to-markdown`) and leaves the markdown as the receipt's
/// preview.
pub fn markdown_call(meta: &AttachmentMeta, function: &str) -> String {
    let payload = if function == DOCUMENT_TO_MARKDOWN {
        format!(r#"{{"file_name":{},"max_chars":0}}"#, quoted(&meta.name))
    } else {
        r#"{"max_chars":0}"#.to_string()
    };
    format!(
        r#"fp::pipe {{"through":[{},{{"function":"{function}","payload":{payload},"into":"/bytes_base64"}},{{"function":"fp::get","payload":{{"path":"/body/text"}}}}],"preview_chars":{PREVIEW_CHARS}}}"#,
        fetch_steps(meta),
    )
}

/// `fp::pipe` that writes the bytes to `saved_file_name(name)` in the
/// working folder of the session that runs it.
pub fn save_call(meta: &AttachmentMeta) -> String {
    format!(
        r#"fp::pipe {{"through":[{},{{"function":"shell::exec","payload":{{"command":"sh","args":["-c","base64 -d > \"$0\"",{}]}},"into":"/stdin"}}]}}"#,
        fetch_steps(meta),
        quoted(&saved_file_name(&meta.name)),
    )
}

fn markdown_sentence(
    meta: &AttachmentMeta,
    function: &str,
    worker: &str,
    f: &RecipeFunctions,
) -> String {
    let available = if function == PDF_TO_MARKDOWN {
        f.pdf_to_markdown
    } else {
        f.document_to_markdown
    };
    if !available {
        return format!(
            "Converting it needs {function}, which this engine does not have: install the {worker} worker."
        );
    }
    format!(
        "Read it as markdown with {}: the receipt's value_preview holds the first {PREVIEW_CHARS} characters; for the next ones add {{\"function\":\"fp::drop\",\"payload\":{{\"n\":{PREVIEW_CHARS}}}}} as the last step (n {} after that, and so on).",
        markdown_call(meta, function),
        PREVIEW_CHARS * 2,
    )
}

fn save_sentence(meta: &AttachmentMeta, lead: &str, f: &RecipeFunctions) -> String {
    if !f.shell_exec {
        return "Saving it into your working folder needs shell::exec, which this engine does not have: install the ide worker.".to_string();
    }
    format!(
        "{lead} {} writes {} there.",
        save_call(meta),
        quoted(&saved_file_name(&meta.name)),
    )
}

/// The paragraph `session::read-attachment` answers for a file that is not
/// text: the exact calls that read or save it on this engine, or which
/// worker to install when a call's function is missing. `None` for text.
pub fn how_to_read(
    kind: AttachmentKind,
    meta: &AttachmentMeta,
    f: &RecipeFunctions,
) -> Option<String> {
    let name = quoted(&meta.name);
    let opening = match kind {
        AttachmentKind::Text => return None,
        AttachmentKind::Pdf => format!("{name} is a PDF."),
        AttachmentKind::Document => format!("{name} is an office document."),
        AttachmentKind::Image => format!(
            "{name} is an image: when you read images, it is in the user message as an image."
        ),
        AttachmentKind::Binary => format!("{name} is not text ({}).", meta.mime),
    };
    if !f.fp_pipe {
        return Some(format!(
            "{opening} Every way to read it goes through fp::pipe, which this engine does not have: install the fp worker."
        ));
    }
    let mut sentences = vec![opening];
    match kind {
        AttachmentKind::Pdf => {
            sentences.push(markdown_sentence(meta, PDF_TO_MARKDOWN, "pdf", f));
            sentences.push(save_sentence(
                meta,
                "To open it with your own tools, save it into your working folder:",
                f,
            ));
        }
        AttachmentKind::Document => {
            sentences.push(markdown_sentence(meta, DOCUMENT_TO_MARKDOWN, "document", f));
            sentences.push(save_sentence(
                meta,
                "To open it with your own tools, save it into your working folder:",
                f,
            ));
        }
        AttachmentKind::Image => {
            sentences.push(save_sentence(
                meta,
                "To open it with your own tools, save it into your working folder:",
                f,
            ));
        }
        AttachmentKind::Binary => {
            sentences.push(save_sentence(
                meta,
                "Save it into your working folder and open it with your own tools (unzip an archive, run a script over data):",
                f,
            ));
        }
        AttachmentKind::Text => unreachable!("text returned above"),
    }
    Some(sentences.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(name: &str, mime: &str) -> AttachmentMeta {
        AttachmentMeta {
            attachment_id: "a_1".into(),
            session_id: "s_1".into(),
            name: name.into(),
            mime: mime.into(),
            size: 1_234_567,
            sha256: "00".into(),
            created_at: 0,
        }
    }

    #[test]
    fn kinds_follow_the_ade_order() {
        use AttachmentKind::*;
        let cases = [
            ("report.pdf", "application/octet-stream", Pdf),
            ("scan", "application/pdf", Pdf),
            ("budget.xlsx", "application/vnd.ms-excel", Document),
            ("data.csv", "text/csv", Document),
            ("logo.svg", "image/svg+xml", Text),
            ("drawing", "image/svg+xml", Text),
            ("page.html", "text/html", Text),
            ("feed", "application/xml", Text),
            ("photo.JPG", "", Image),
            ("shot", "image/png", Image),
            ("notes.md", "application/octet-stream", Text),
            ("readme", "text/plain; charset=utf-8", Text),
            ("config", "application/json", Text),
            ("bundle.zip", "application/zip", Binary),
            (".env", "application/octet-stream", Binary),
            ("noext", "application/octet-stream", Binary),
        ];
        for (name, mime, want) in cases {
            assert_eq!(AttachmentKind::of(name, mime), want, "{name} {mime}");
        }
    }

    #[test]
    fn text_that_is_not_utf8_is_binary() {
        assert_eq!(
            AttachmentKind::of_bytes("notes.txt", "text/plain", b"caf\xe9"),
            AttachmentKind::Binary
        );
        assert_eq!(
            AttachmentKind::of_bytes("notes.txt", "text/plain", "café".as_bytes()),
            AttachmentKind::Text
        );
        // Only text is checked: a PDF stays a PDF whatever its bytes.
        assert_eq!(
            AttachmentKind::of_bytes("r.pdf", "application/pdf", b"\xff\xfe"),
            AttachmentKind::Pdf
        );
    }

    #[test]
    fn kind_serializes_lowercase() {
        assert_eq!(
            serde_json::to_value(AttachmentKind::Document).unwrap(),
            serde_json::json!("document")
        );
        assert_eq!(AttachmentKind::Pdf.label(), "PDF");
        assert_eq!(AttachmentKind::Binary.label(), "File");
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1024), "1 KB");
        assert_eq!(human_size(1_234_567), "1.2 MB");
        assert_eq!(human_size(345_678), "338 KB");
        assert_eq!(human_size(11 * 1024 * 1024), "11 MB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3 GB");
    }

    #[test]
    fn text_windows_count_characters_and_point_at_the_next_one() {
        let text = "héllo wörld"; // 11 characters, 13 bytes
        let w = text_window(text, None, Some(5));
        assert_eq!(w.text, "héllo");
        assert_eq!(
            (w.offset, w.next_offset, w.total_chars, w.truncated),
            (0, Some(5), 11, true)
        );
        let w = text_window(text, Some(5), Some(100));
        assert_eq!(w.text, " wörld");
        assert_eq!((w.offset, w.next_offset, w.truncated), (5, None, false));
        let w = text_window(text, Some(99), None);
        assert_eq!((w.text.as_str(), w.offset, w.next_offset), ("", 11, None));
        // max_chars is clamped to 1..=100000 and defaults to 20000.
        assert_eq!(text_window(text, None, Some(0)).text, "h");
        let long = "x".repeat(150_000);
        assert_eq!(text_window(&long, None, None).next_offset, Some(20_000));
        assert_eq!(
            text_window(&long, None, Some(500_000)).next_offset,
            Some(100_000)
        );
    }

    #[test]
    fn saved_names_stay_in_the_folder() {
        assert_eq!(saved_file_name("bundle.zip"), "bundle.zip");
        assert_eq!(saved_file_name("../../etc/passwd"), ".._.._etc_passwd");
        assert_eq!(saved_file_name("a\nb"), "a_b");
        assert_eq!(saved_file_name(" .. "), "attachment");
        assert_eq!(saved_file_name(""), "attachment");
    }

    #[test]
    fn text_has_no_recipe() {
        assert_eq!(
            how_to_read(
                AttachmentKind::Text,
                &meta("n.md", "text/markdown"),
                &RecipeFunctions::ALL
            ),
            None
        );
    }

    #[test]
    fn a_pdf_recipe_pipes_the_bytes_into_pdf_to_markdown_and_offers_saving() {
        let text = how_to_read(
            AttachmentKind::Pdf,
            &meta("report.pdf", "application/pdf"),
            &RecipeFunctions::ALL,
        )
        .unwrap();
        assert!(text.starts_with("\"report.pdf\" is a PDF. Read it as markdown with fp::pipe "));
        assert!(text.contains(
            r#"fp::pipe {"through":[{"function":"session::get-attachment","payload":{"session_id":"s_1","attachment_id":"a_1"}},{"function":"fp::get","payload":{"path":"/data"}},{"function":"pdf::to-markdown","payload":{"max_chars":0},"into":"/bytes_base64"},{"function":"fp::get","payload":{"path":"/body/text"}}],"preview_chars":8000}"#
        ));
        assert!(text.contains(r#"{"function":"fp::drop","payload":{"n":8000}}"#));
        assert!(text.contains(
            r#"{"function":"shell::exec","payload":{"command":"sh","args":["-c","base64 -d > \"$0\"","report.pdf"]},"into":"/stdin"}]}"#
        ));
        assert!(text.ends_with("writes \"report.pdf\" there."));
    }

    #[test]
    fn every_pipe_in_a_recipe_is_valid_json() {
        for (kind, name, mime) in [
            (AttachmentKind::Pdf, "r \"q\".pdf", "application/pdf"),
            (
                AttachmentKind::Document,
                "b.docx",
                "application/octet-stream",
            ),
            (AttachmentKind::Binary, "a b.zip", "application/zip"),
        ] {
            let m = meta(name, mime);
            let calls = [
                save_call(&m),
                markdown_call(&m, PDF_TO_MARKDOWN),
                markdown_call(&m, DOCUMENT_TO_MARKDOWN),
            ];
            for call in calls {
                let json = call.strip_prefix("fp::pipe ").expect("fp::pipe call");
                let parsed: serde_json::Value = serde_json::from_str(json).expect("valid JSON");
                assert_eq!(parsed["through"][0]["payload"]["attachment_id"], "a_1");
            }
            assert!(how_to_read(kind, &m, &RecipeFunctions::ALL).is_some());
        }
        let doc = markdown_call(&meta("b.docx", ""), DOCUMENT_TO_MARKDOWN);
        let parsed: serde_json::Value =
            serde_json::from_str(doc.strip_prefix("fp::pipe ").unwrap()).unwrap();
        assert_eq!(parsed["through"][2]["payload"]["file_name"], "b.docx");
    }

    #[test]
    fn images_and_binaries_are_saved_into_the_working_folder() {
        let image = how_to_read(
            AttachmentKind::Image,
            &meta("p.png", "image/png"),
            &RecipeFunctions::ALL,
        )
        .unwrap();
        assert!(image.contains("it is in the user message as an image"));
        assert!(image.contains("save it into your working folder"));
        let zip = how_to_read(
            AttachmentKind::Binary,
            &meta("bundle.zip", "application/zip"),
            &RecipeFunctions::ALL,
        )
        .unwrap();
        assert!(zip.starts_with(
            "\"bundle.zip\" is not text (application/zip). Save it into your working folder"
        ));
        assert!(!zip.contains("to-markdown"));
    }

    #[test]
    fn a_missing_worker_is_named() {
        let none = RecipeFunctions {
            fp_pipe: true,
            pdf_to_markdown: false,
            document_to_markdown: false,
            shell_exec: false,
        };
        let pdf = how_to_read(
            AttachmentKind::Pdf,
            &meta("r.pdf", "application/pdf"),
            &none,
        )
        .unwrap();
        assert!(pdf.contains(
            "needs pdf::to-markdown, which this engine does not have: install the pdf worker."
        ));
        assert!(pdf.contains(
            "needs shell::exec, which this engine does not have: install the ide worker."
        ));
        let doc = how_to_read(AttachmentKind::Document, &meta("b.docx", ""), &none).unwrap();
        assert!(doc.contains("install the document worker"));
        let no_fp = RecipeFunctions {
            fp_pipe: false,
            ..RecipeFunctions::ALL
        };
        let zip = how_to_read(
            AttachmentKind::Binary,
            &meta("z.zip", "application/zip"),
            &no_fp,
        )
        .unwrap();
        assert!(zip.ends_with("install the fp worker."));
        assert!(!zip.contains("fp::pipe {"));
    }
}
