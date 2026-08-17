//! The wire vocabulary for formats, and how it maps onto the converter.
//!
//! The converter's own `Format` is a Rust enum with no serde derives, so it
//! cannot be the wire type; this module owns the names a caller sees and the
//! translation in both directions. Keeping them separate is also what lets the
//! wire stay stable when the converter adds a variant.
//!
//! Two things ride along with the name because a caller needs them and would
//! otherwise hard-code a match of its own: the `family` a format belongs to
//! (what the document IS — prose, a sheet, a deck), and whether the format was
//! recognised from the bytes or only from the file extension. The distinction
//! matters: a mislabelled `.txt` that is really a Word file converts fine, and
//! a `.csv` cannot be recognised from content at all, so "detected from the
//! extension" is a weaker claim that a caller may want to act on.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A format this worker converts. The names are the wire vocabulary: stable,
/// lowercase, and independent of the file extension that named them (`.docm`
/// is `docx`, `.xlsb` is `excel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// Binary Word 97-2003 (`.doc`).
    Doc,
    /// WordprocessingML (`.docx`, `.docm`).
    Docx,
    /// OpenDocument Text (`.odt`).
    Odt,
    /// Rich Text Format (`.rtf`).
    Rtf,
    /// Binary PowerPoint 97-2003 (`.ppt`, `.pps`, `.pot`).
    Ppt,
    /// PresentationML (`.pptx`, `.pptm`, `.ppsx`, `.ppsm`).
    Pptx,
    /// OpenDocument Presentation (`.odp`).
    Odp,
    /// Excel workbooks in every container (`.xlsx`, `.xlsm`, `.xlsb`, `.xls`).
    Excel,
    /// OpenDocument Spreadsheet (`.ods`).
    Ods,
    /// Delimiter-separated text (`.csv`).
    Csv,
    /// EPUB 2 and 3 (`.epub`).
    Epub,
    /// Portable Document Format (`.pdf`).
    Pdf,
}

/// What the document is, rather than which program wrote it.
///
/// A caller routing a mixed bag of attachments cares that a file is a
/// spreadsheet, not that it is `.ods` rather than `.xlsx`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    /// Prose: Word, OpenDocument Text, RTF.
    Prose,
    /// Rows and columns: Excel, OpenDocument Spreadsheet, CSV.
    Spreadsheet,
    /// Slides: PowerPoint, OpenDocument Presentation.
    Presentation,
    /// A book: EPUB.
    Book,
    /// PDF, which is its own family because it is the one format with a
    /// dedicated worker and a page-level OCR decision.
    Pdf,
}

/// How the format was arrived at, weakest claim last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DetectedFrom {
    /// The caller named it, and the bytes were not consulted.
    Requested,
    /// The signature the format's specification designates (PDF header, RTF
    /// open group, OLE stream names, ZIP package mimetype).
    Content,
    /// The file extension only. CSV carries no signature, so this is the only
    /// way it is ever recognised; for any other format it means the content
    /// did not match anything known.
    Extension,
}

impl Format {
    /// The converter variant this name selects.
    pub fn to_anydoc(self) -> anydoc::Format {
        match self {
            Format::Doc => anydoc::Format::Doc,
            Format::Docx => anydoc::Format::Docx,
            Format::Odt => anydoc::Format::Odt,
            Format::Rtf => anydoc::Format::Rtf,
            Format::Ppt => anydoc::Format::Ppt,
            Format::Pptx => anydoc::Format::Pptx,
            Format::Odp => anydoc::Format::Odp,
            Format::Excel => anydoc::Format::Excel,
            Format::Ods => anydoc::Format::Ods,
            Format::Csv => anydoc::Format::Csv,
            Format::Epub => anydoc::Format::Epub,
            Format::Pdf => anydoc::Format::Pdf,
        }
    }

    /// The wire name for a converter variant.
    pub fn from_anydoc(format: anydoc::Format) -> Self {
        match format {
            anydoc::Format::Doc => Format::Doc,
            anydoc::Format::Docx => Format::Docx,
            anydoc::Format::Odt => Format::Odt,
            anydoc::Format::Rtf => Format::Rtf,
            anydoc::Format::Ppt => Format::Ppt,
            anydoc::Format::Pptx => Format::Pptx,
            anydoc::Format::Odp => Format::Odp,
            anydoc::Format::Excel => Format::Excel,
            anydoc::Format::Ods => Format::Ods,
            anydoc::Format::Csv => Format::Csv,
            anydoc::Format::Epub => Format::Epub,
            anydoc::Format::Pdf => Format::Pdf,
        }
    }

    pub fn family(self) -> Family {
        match self {
            Format::Doc | Format::Docx | Format::Odt | Format::Rtf => Family::Prose,
            Format::Excel | Format::Ods | Format::Csv => Family::Spreadsheet,
            Format::Ppt | Format::Pptx | Format::Odp => Family::Presentation,
            Format::Epub => Family::Book,
            Format::Pdf => Family::Pdf,
        }
    }

    /// `true` when the format parses into the document model, which is what
    /// `document::extract-assets` walks. A PDF converts straight to markdown
    /// and never builds a model, so it has no assets to hand back here.
    pub fn has_document_model(self) -> bool {
        self != Format::Pdf
    }

    /// `true` when the format can embed an image or object at all.
    ///
    /// Counting assets costs a second parse of the whole document, and a CSV is
    /// rows of text with nowhere to put a picture. Skipping it there is the
    /// difference between one parse and two on the cheapest format people
    /// attach.
    pub fn carries_assets(self) -> bool {
        self.has_document_model() && self != Format::Csv
    }
}

/// Resolve the format for a document, and say how the answer was reached.
///
/// Order is deliberate: an explicit request wins because the caller may know
/// something the bytes do not say; content beats the extension because a
/// mislabelled file is common and a wrong extension is not worth failing over;
/// the extension is the last resort, and the only route for CSV.
/// [`resolve`], with the refusal a handler owes its caller when nothing
/// matched.
///
/// `document::detect` wants the bare `Option` — "not a document I read" is its
/// answer, not a failure. Every function that goes on to convert wants the same
/// sentence, so it lives here rather than being written twice and drifting.
pub fn resolve_or_explain(
    requested: Option<Format>,
    bytes: &[u8],
    file_name: Option<&str>,
    label: &str,
) -> Result<(Format, DetectedFrom), String> {
    resolve(requested, bytes, file_name).ok_or_else(|| {
        format!(
            "{label} is not a document this worker reads: nothing in its content matched a known \
             format, and its name did not name one either. Pass `format` if you know what it is."
        )
    })
}

pub fn resolve(
    requested: Option<Format>,
    bytes: &[u8],
    file_name: Option<&str>,
) -> Option<(Format, DetectedFrom)> {
    if let Some(format) = requested {
        return Some((format, DetectedFrom::Requested));
    }
    if let Some(format) = anydoc::Format::from_bytes(bytes) {
        return Some((Format::from_anydoc(format), DetectedFrom::Content));
    }
    let extension = file_name
        .and_then(|name| std::path::Path::new(name).extension())
        .and_then(|ext| ext.to_str())?;
    anydoc::Format::from_extension(extension)
        .map(|format| (Format::from_anydoc(format), DetectedFrom::Extension))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every wire name must survive the round trip through the converter's
    /// enum. A missed arm here silently converts one format as another.
    #[test]
    fn every_format_round_trips_through_the_converter() {
        for format in [
            Format::Doc,
            Format::Docx,
            Format::Odt,
            Format::Rtf,
            Format::Ppt,
            Format::Pptx,
            Format::Odp,
            Format::Excel,
            Format::Ods,
            Format::Csv,
            Format::Epub,
            Format::Pdf,
        ] {
            assert_eq!(Format::from_anydoc(format.to_anydoc()), format);
        }
    }

    #[test]
    fn wire_names_are_lowercase_and_stable() {
        assert_eq!(
            serde_json::to_string(&Format::Docx).expect("serializes"),
            "\"docx\""
        );
        assert_eq!(
            serde_json::to_string(&Format::Excel).expect("serializes"),
            "\"excel\""
        );
        assert_eq!(
            serde_json::to_string(&Family::Spreadsheet).expect("serializes"),
            "\"spreadsheet\""
        );
    }

    /// A container variant is not its own wire name: `.docm` is a Word file
    /// and `.xlsb` is a workbook, and a caller matching on the extension it
    /// sent would otherwise have to know every alias.
    #[test]
    fn container_aliases_collapse_onto_one_name() {
        let by_extension = |ext: &str| {
            anydoc::Format::from_extension(ext)
                .map(Format::from_anydoc)
                .expect("known extension")
        };
        assert_eq!(by_extension("docm"), Format::Docx);
        assert_eq!(by_extension("xlsb"), Format::Excel);
        assert_eq!(by_extension("ppsm"), Format::Pptx);
    }

    #[test]
    fn an_explicit_format_skips_detection() {
        let (format, how) = resolve(Some(Format::Csv), b"not,a,csv,signature", None)
            .expect("an explicit format always resolves");
        assert_eq!(format, Format::Csv);
        assert_eq!(how, DetectedFrom::Requested);
    }

    /// The PDF header is the cheapest real signature to assert against, and it
    /// proves detection runs on content before the extension is consulted.
    #[test]
    fn content_beats_a_lying_extension() {
        let (format, how) =
            resolve(None, b"%PDF-1.7\n", Some("report.docx")).expect("content is recognised");
        assert_eq!(format, Format::Pdf);
        assert_eq!(how, DetectedFrom::Content);
    }

    /// CSV carries no signature. Without the extension fallback a spreadsheet
    /// export would be unreadable, which is the common case for exported data.
    #[test]
    fn a_signature_less_format_falls_back_to_the_extension() {
        let (format, how) =
            resolve(None, b"a,b,c\n1,2,3\n", Some("rows.csv")).expect("extension resolves it");
        assert_eq!(format, Format::Csv);
        assert_eq!(how, DetectedFrom::Extension);
    }

    #[test]
    fn nothing_recognisable_resolves_to_nothing() {
        assert!(resolve(None, b"\x00\x01\x02", Some("mystery.bin")).is_none());
        assert!(resolve(None, b"\x00\x01\x02", None).is_none());
    }

    /// The document model is what `document::extract-assets` walks, and the
    /// converter has no model form for a PDF.
    #[test]
    fn pdf_has_no_document_model() {
        assert!(!Format::Pdf.has_document_model());
        assert!(Format::Pptx.has_document_model());
    }
}
