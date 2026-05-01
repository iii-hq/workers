//! Extract text from PDF and Word documents.
//!
//! Used by tools like `document::extract` to ingest user-uploaded files for
//! agent context. The output text is always UTF-8; metadata always carries
//! `byte_size` and `sniffed_mime`. Page count is best-effort and only
//! populated for PDFs.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentFormat {
    Pdf,
    Docx,
    Auto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentExtract {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_count: Option<u32>,
    pub metadata: serde_json::Value,
    pub detected_format: DocumentFormat,
}

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("parse error: {0}")]
    Parse(String),
}

/// Sniff the file format from magic bytes. Returns [`DocumentFormat::Auto`]
/// when neither PDF nor DOCX can be detected.
pub fn detect_format(bytes: &[u8]) -> DocumentFormat {
    if bytes.starts_with(b"%PDF-") {
        return DocumentFormat::Pdf;
    }
    let kind = infer::get(bytes);
    if let Some(k) = kind {
        let mime = k.mime_type();
        if mime == "application/pdf" {
            return DocumentFormat::Pdf;
        }
        let docx_mime = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
        if mime == docx_mime {
            return DocumentFormat::Docx;
        }
        if mime == "application/zip" && looks_like_docx_zip(bytes) {
            return DocumentFormat::Docx;
        }
    } else if looks_like_docx_zip(bytes) {
        return DocumentFormat::Docx;
    }
    DocumentFormat::Auto
}

fn looks_like_docx_zip(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"PK\x03\x04") {
        return false;
    }
    let needle = b"word/document.xml";
    bytes.windows(needle.len()).any(|window| window == needle)
}

/// Extract a document from disk. The file is read into memory, then
/// dispatched to the in-memory extractor.
pub async fn extract(
    file_path: &Path,
    format: DocumentFormat,
) -> Result<DocumentExtract, ExtractError> {
    let bytes = tokio::fs::read(file_path).await?;
    extract_bytes(&bytes, format).await
}

/// Extract a document from raw bytes.
pub async fn extract_bytes(
    bytes: &[u8],
    format: DocumentFormat,
) -> Result<DocumentExtract, ExtractError> {
    let resolved = match format {
        DocumentFormat::Auto => detect_format(bytes),
        other => other,
    };
    match resolved {
        DocumentFormat::Pdf => extract_pdf(bytes).await,
        DocumentFormat::Docx => extract_docx(bytes).await,
        DocumentFormat::Auto => Err(ExtractError::UnsupportedFormat(
            "could not detect document format".into(),
        )),
    }
}

async fn extract_pdf(bytes: &[u8]) -> Result<DocumentExtract, ExtractError> {
    let owned = bytes.to_vec();
    let byte_size = owned.len();
    let result = tokio::task::spawn_blocking(move || pdf_extract::extract_text_from_mem(&owned))
        .await
        .map_err(|e| ExtractError::Parse(format!("join: {e}")))?
        .map_err(|e| ExtractError::Parse(e.to_string()))?;
    let page_count = count_pdf_pages(bytes);
    let metadata = serde_json::json!({
        "byte_size": byte_size,
        "sniffed_mime": "application/pdf",
    });
    Ok(DocumentExtract {
        text: result,
        page_count,
        metadata,
        detected_format: DocumentFormat::Pdf,
    })
}

/// Best-effort page count: walk the buffer looking for `/Type /Page` markers,
/// skipping the `/Type /Pages` catalog node.
fn count_pdf_pages(bytes: &[u8]) -> Option<u32> {
    let mut count: u32 = 0;
    for needle in [b"/Type /Page".as_slice(), b"/Type/Page".as_slice()] {
        for (i, w) in bytes.windows(needle.len()).enumerate() {
            if w != needle {
                continue;
            }
            let next = bytes.get(i + needle.len()).copied().unwrap_or(0);
            if next == b's' {
                continue;
            }
            count += 1;
        }
    }
    if count == 0 {
        None
    } else {
        Some(count)
    }
}

async fn extract_docx(bytes: &[u8]) -> Result<DocumentExtract, ExtractError> {
    let owned = bytes.to_vec();
    let byte_size = owned.len();
    let text = tokio::task::spawn_blocking(move || extract_docx_text(&owned))
        .await
        .map_err(|e| ExtractError::Parse(format!("join: {e}")))??;
    let metadata = serde_json::json!({
        "byte_size": byte_size,
        "sniffed_mime": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    });
    Ok(DocumentExtract {
        text,
        page_count: None,
        metadata,
        detected_format: DocumentFormat::Docx,
    })
}

fn extract_docx_text(bytes: &[u8]) -> Result<String, ExtractError> {
    let doc = docx_rs::read_docx(bytes).map_err(|e| ExtractError::Parse(e.to_string()))?;
    let mut out = String::new();
    walk_children(&doc.document.children, &mut out);
    Ok(out.trim_end().to_string())
}

fn walk_children(children: &[docx_rs::DocumentChild], out: &mut String) {
    for child in children {
        match child {
            docx_rs::DocumentChild::Paragraph(p) => {
                append_paragraph(p, out);
                out.push('\n');
            }
            docx_rs::DocumentChild::Table(t) => {
                for row in &t.rows {
                    let docx_rs::TableChild::TableRow(row) = row;
                    for cell in &row.cells {
                        let docx_rs::TableRowChild::TableCell(cell) = cell;
                        for c in &cell.children {
                            if let docx_rs::TableCellContent::Paragraph(p) = c {
                                append_paragraph(p, out);
                                out.push('\t');
                            }
                        }
                    }
                    out.push('\n');
                }
            }
            _ => {}
        }
    }
}

fn append_paragraph(p: &docx_rs::Paragraph, out: &mut String) {
    for child in &p.children {
        if let docx_rs::ParagraphChild::Run(run) = child {
            for rc in &run.children {
                if let docx_rs::RunChild::Text(t) = rc {
                    out.push_str(&t.text);
                }
            }
        }
    }
}

/// Registered function ids exposed by [`register_with_iii`].
pub mod function_ids {
    pub const EXTRACT: &str = "document::extract";
}

/// Maximum bytes the `document::extract` iii function will read by default.
/// Callers can override with the `max_bytes` payload field.
pub const DEFAULT_MAX_BYTES: u64 = 25 * 1024 * 1024;

/// Register the `document::extract` iii function on `iii`.
///
/// # Payload
///
/// `{ "path": str, "max_bytes": u64? }` — `max_bytes` defaults to
/// [`DEFAULT_MAX_BYTES`]. The format is sniffed from the file's magic bytes.
///
/// Returns `{ "text": str, "page_count": u32?, "metadata": Value,
/// "detected_format": "pdf" | "docx" | "auto" }`.
pub fn register_with_iii(iii: &iii_sdk::III) -> DocumentFunctionRefs {
    use iii_sdk::{IIIError, RegisterFunctionMessage};

    let f = iii.register_function((
        RegisterFunctionMessage::with_id(function_ids::EXTRACT.into())
            .with_description("Extract UTF-8 text from a PDF or DOCX file on disk".into()),
        move |payload: serde_json::Value| async move {
            let path = payload
                .get("path")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| IIIError::Handler("missing required field: path".into()))?;
            let max_bytes = payload
                .get("max_bytes")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(DEFAULT_MAX_BYTES);

            let path = std::path::Path::new(path);
            let metadata = tokio::fs::metadata(path)
                .await
                .map_err(|e| IIIError::Handler(format!("stat: {e}")))?;
            if metadata.len() > max_bytes {
                return Err(IIIError::Handler(format!(
                    "file size {} exceeds max_bytes {max_bytes}",
                    metadata.len()
                )));
            }
            let result = extract(path, DocumentFormat::Auto)
                .await
                .map_err(|e| IIIError::Handler(e.to_string()))?;
            serde_json::to_value(result).map_err(|e| IIIError::Handler(e.to_string()))
        },
    ));

    DocumentFunctionRefs { refs: vec![f] }
}

/// Handle returned by [`register_with_iii`].
pub struct DocumentFunctionRefs {
    refs: Vec<iii_sdk::FunctionRef>,
}

impl DocumentFunctionRefs {
    pub fn unregister_all(self) {
        for f in self.refs {
            f.unregister();
        }
    }

    pub fn len(&self) -> usize {
        self.refs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.refs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-crafted minimal PDF (~280 bytes) containing a single page with text "hi".
    const TINY_PDF: &[u8] = b"%PDF-1.1\n\
1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n\
2 0 obj<</Type/Pages/Count 1/Kids[3 0 R]>>endobj\n\
3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>endobj\n\
4 0 obj<</Length 44>>stream\n\
BT /F1 24 Tf 50 100 Td (hi) Tj ET\n\
endstream endobj\n\
5 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj\n\
xref\n\
0 6\n\
0000000000 65535 f\n\
0000000009 00000 n\n\
0000000052 00000 n\n\
0000000101 00000 n\n\
0000000192 00000 n\n\
0000000260 00000 n\n\
trailer<</Size 6/Root 1 0 R>>\n\
startxref\n\
320\n\
%%EOF\n";

    #[test]
    fn detect_format_identifies_pdf_magic() {
        assert_eq!(detect_format(b"%PDF-1.4 ..."), DocumentFormat::Pdf);
    }

    #[test]
    fn detect_format_identifies_docx_zip() {
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(b"PK\x03\x04");
        bytes.extend_from_slice(&[0u8; 32]);
        bytes.extend_from_slice(b"word/document.xml");
        bytes.extend_from_slice(&[0u8; 32]);
        assert_eq!(detect_format(&bytes), DocumentFormat::Docx);
    }

    #[test]
    fn detect_format_returns_auto_for_garbage() {
        assert_eq!(detect_format(b"hello world"), DocumentFormat::Auto);
    }

    #[tokio::test]
    async fn extract_bytes_unsupported_format_returns_error() {
        let result = extract_bytes(b"hello world", DocumentFormat::Auto).await;
        assert!(matches!(result, Err(ExtractError::UnsupportedFormat(_))));
    }

    #[tokio::test]
    async fn extract_bytes_pdf_roundtrip() {
        let result = extract_bytes(TINY_PDF, DocumentFormat::Pdf).await;
        match result {
            Ok(extract) => {
                assert_eq!(extract.detected_format, DocumentFormat::Pdf);
                let mime = extract
                    .metadata
                    .get("sniffed_mime")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                assert_eq!(mime, "application/pdf");
                let byte_size = extract
                    .metadata
                    .get("byte_size")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_default();
                assert_eq!(byte_size as usize, TINY_PDF.len());
            }
            Err(e) => {
                assert!(matches!(e, ExtractError::Parse(_)), "unexpected: {e:?}");
            }
        }
    }

    #[test]
    fn page_count_finds_page_marker() {
        assert_eq!(count_pdf_pages(TINY_PDF), Some(1));
        assert_eq!(count_pdf_pages(b"no markers"), None);
    }

    #[test]
    fn document_extract_serializes() {
        let ex = DocumentExtract {
            text: "hello".into(),
            page_count: Some(1),
            metadata: serde_json::json!({"k": "v"}),
            detected_format: DocumentFormat::Pdf,
        };
        let s = serde_json::to_string(&ex).unwrap();
        let back: DocumentExtract = serde_json::from_str(&s).unwrap();
        assert_eq!(ex, back);
    }

    #[tokio::test]
    #[ignore = "supply a sample.docx in tests/fixtures/ to enable"]
    async fn extract_docx_fixture() {
        let path = std::path::Path::new("tests/fixtures/sample.docx");
        let bytes = std::fs::read(path).expect("fixture present");
        let extract = extract_bytes(&bytes, DocumentFormat::Docx).await.unwrap();
        assert_eq!(extract.detected_format, DocumentFormat::Docx);
        assert!(!extract.text.is_empty());
    }
}
