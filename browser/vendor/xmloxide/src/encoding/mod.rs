//! Encoding detection and transcoding.
//!
//! Implements BOM sniffing and XML declaration encoding detection per
//! XML 1.0 Section 4.3.3 and Appendix F, bridging to `encoding_rs` for character
//! encoding conversion.
//!
//! # Encoding Detection Strategy
//!
//! 1. Check for a Byte Order Mark (BOM) at the start of the input.
//! 2. If a BOM is found, use the indicated encoding and skip the BOM bytes.
//! 3. If no BOM is found, default to UTF-8 (per the XML specification).
//! 4. After initial decoding, inspect the XML declaration's `encoding=` attribute
//!    to confirm or override the detected encoding.

use std::fmt;

/// An error that occurs during encoding detection or transcoding.
#[derive(Debug, Clone)]
pub struct EncodingError {
    /// A human-readable description of the encoding error.
    pub message: String,
}

impl EncodingError {
    /// Creates a new `EncodingError` with the given message.
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for EncodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "encoding error: {}", self.message)
    }
}

impl std::error::Error for EncodingError {}

/// Detects the encoding of an XML byte stream by inspecting the Byte Order Mark.
///
/// Returns a tuple of (encoding name, number of BOM bytes to skip). The encoding
/// name is an IANA charset name suitable for passing to `encoding_rs`.
///
/// Per XML 1.0 Appendix F, the BOM detection order is:
/// - `EF BB BF` -> UTF-8
/// - `FE FF`    -> UTF-16 BE
/// - `FF FE`    -> UTF-16 LE
/// - No BOM     -> UTF-8 (default per XML spec)
///
/// # Examples
///
/// ```
/// use xmloxide::encoding::detect_encoding;
///
/// let (enc, skip) = detect_encoding(b"\xEF\xBB\xBFhello");
/// assert_eq!(enc, "UTF-8");
/// assert_eq!(skip, 3);
///
/// let (enc, skip) = detect_encoding(b"<root/>");
/// assert_eq!(enc, "UTF-8");
/// assert_eq!(skip, 0);
/// ```
#[must_use]
pub fn detect_encoding(bytes: &[u8]) -> (&'static str, usize) {
    if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
        ("UTF-8", 3)
    } else if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        ("UTF-16BE", 2)
    } else if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        ("UTF-16LE", 2)
    } else {
        ("UTF-8", 0)
    }
}

/// Transcodes a byte slice from the named encoding into a UTF-8 `String`.
///
/// Uses `encoding_rs::Encoding::for_label` to look up the encoding by its IANA
/// name (case-insensitive). Returns an error if the encoding is unknown or if
/// the input contains malformed byte sequences.
///
/// # Errors
///
/// Returns `EncodingError` if the encoding name is not recognized or if
/// transcoding fails due to malformed input bytes.
///
/// # Examples
///
/// ```
/// use xmloxide::encoding::transcode;
///
/// let result = transcode(b"hello", "UTF-8").unwrap();
/// assert_eq!(result, "hello");
/// ```
pub fn transcode(bytes: &[u8], encoding_name: &str) -> Result<String, EncodingError> {
    let encoding = encoding_rs::Encoding::for_label(encoding_name.as_bytes())
        .ok_or_else(|| EncodingError::new(format!("unsupported encoding: {encoding_name}")))?;

    let (result, _used_encoding, had_errors) = encoding.decode(bytes);
    if had_errors {
        return Err(EncodingError::new(format!(
            "malformed byte sequence for encoding {encoding_name}"
        )));
    }
    Ok(result.into_owned())
}

/// Extracts the `encoding` attribute value from an XML declaration in the given string.
///
/// This performs a lightweight scan of the first line to find a pattern like
/// `encoding="..."` or `encoding='...'` without running the full XML parser.
/// Returns `None` if no XML declaration or no encoding attribute is found.
fn extract_xml_decl_encoding(text: &str) -> Option<String> {
    // Only look at the beginning of the document, up to the end of the XML decl.
    let decl_end = text.find("?>")?;
    let decl = &text[..decl_end];

    // Must start with <?xml to be a valid XML declaration
    if !decl.starts_with("<?xml") {
        return None;
    }

    let enc_pos = decl.find("encoding")?;
    let after_enc = &decl[enc_pos + "encoding".len()..];

    // Skip whitespace and '='
    let after_enc = after_enc.trim_start();
    let after_enc = after_enc.strip_prefix('=')?;
    let after_enc = after_enc.trim_start();

    // Extract the quoted value
    let quote = after_enc.as_bytes().first().copied()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let after_quote = &after_enc[1..];
    let end = after_quote.find(quote as char)?;
    Some(after_quote[..end].to_string())
}

/// Decodes raw XML bytes into a UTF-8 string, automatically detecting the encoding.
///
/// This implements the full encoding detection pipeline from XML 1.0 Section 4.3.3:
///
/// 1. Detect the BOM and determine the initial encoding.
/// 2. If the encoding is UTF-8, validate and return the bytes as a string.
/// 3. If non-UTF-8, transcode using `encoding_rs`.
/// 4. After the initial decode, check the XML declaration's `encoding=` attribute.
///    If it specifies a different encoding than what the BOM indicated, re-decode
///    from the original bytes using the declared encoding.
///
/// # Errors
///
/// Returns `EncodingError` if the bytes contain invalid sequences for the
/// detected encoding or if the declared encoding is unsupported.
///
/// # Examples
///
/// ```
/// use xmloxide::encoding::decode_to_utf8;
///
/// let xml = b"<?xml version=\"1.0\"?><root/>";
/// let result = decode_to_utf8(xml).unwrap();
/// assert!(result.contains("<root/>"));
/// ```
pub fn decode_to_utf8(bytes: &[u8]) -> Result<String, EncodingError> {
    let (bom_encoding, bom_skip) = detect_encoding(bytes);
    let content_bytes = &bytes[bom_skip..];

    // Fast path: if the BOM says UTF-8 (or no BOM, which defaults to UTF-8),
    // try to validate directly without transcoding.
    if bom_encoding == "UTF-8" {
        if let Ok(s) = std::str::from_utf8(content_bytes) {
            // Valid UTF-8. Check for an encoding declaration that might
            // indicate a different encoding (unusual but permitted).
            if let Some(declared) = extract_xml_decl_encoding(s) {
                let declared_upper = declared.to_ascii_uppercase();
                if !is_utf8_label(&declared_upper) {
                    return transcode(content_bytes, &declared);
                }
            }
            return Ok(s.to_string());
        }
        // Not valid UTF-8 and no BOM. The XML declaration is required
        // to be in ASCII-compatible bytes, so try to extract the
        // encoding= attribute from the raw bytes interpreted as ASCII.
        // If found, transcode with the declared encoding; otherwise,
        // the input is genuinely malformed UTF-8.
        if let Some(declared) = extract_encoding_from_ascii_bytes(content_bytes) {
            return transcode(content_bytes, &declared);
        }
        return Err(EncodingError::new("input is not valid UTF-8"));
    }

    // Non-UTF-8 BOM encoding: transcode first, then check for a declaration.
    let initial_text = transcode(content_bytes, bom_encoding)?;

    if let Some(declared_encoding) = extract_xml_decl_encoding(&initial_text) {
        let declared_upper = declared_encoding.to_ascii_uppercase();
        let bom_upper = bom_encoding.to_ascii_uppercase();

        let effectively_same = declared_upper == bom_upper
            || (is_utf8_label(&declared_upper) && is_utf8_label(&bom_upper))
            // "UTF-16" is compatible with both "UTF-16BE" and "UTF-16LE" —
            // the BOM determines the actual byte order.
            || (declared_upper == "UTF-16"
                && (bom_upper == "UTF-16BE" || bom_upper == "UTF-16LE"));

        if !effectively_same {
            return transcode(content_bytes, &declared_encoding);
        }
    }

    Ok(initial_text)
}

/// Extracts the `encoding` attribute from raw bytes by treating them as ASCII.
///
/// This is used as a fallback when the input is not valid UTF-8 and has no BOM.
/// Since the XML declaration must be in ASCII-compatible characters, we can scan
/// the bytes directly. Returns `None` if no encoding declaration is found.
fn extract_encoding_from_ascii_bytes(bytes: &[u8]) -> Option<String> {
    // Only scan up to a reasonable limit for the XML declaration (first 200 bytes).
    let limit = bytes.len().min(200);
    let scan = &bytes[..limit];

    // Look for "<?xml" at the start
    if !scan.starts_with(b"<?xml") {
        return None;
    }

    // Find "?>" to delimit the declaration
    let decl_end = scan.windows(2).position(|w| w == b"?>")?;
    let decl = &scan[..decl_end];

    // Find "encoding" within the declaration
    let enc_needle = b"encoding";
    let enc_pos = decl
        .windows(enc_needle.len())
        .position(|w| w == enc_needle)?;
    let after_enc = &decl[enc_pos + enc_needle.len()..];

    // Skip whitespace and '='
    let after_enc = skip_ascii_whitespace(after_enc);
    if after_enc.first() != Some(&b'=') {
        return None;
    }
    let after_eq = skip_ascii_whitespace(&after_enc[1..]);

    // Extract quoted value
    let quote = *after_eq.first()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let after_quote = &after_eq[1..];
    let end = after_quote.iter().position(|&b| b == quote)?;
    let encoding_bytes = &after_quote[..end];

    // The encoding name must be ASCII
    if encoding_bytes.iter().all(u8::is_ascii) {
        Some(String::from_utf8_lossy(encoding_bytes).into_owned())
    } else {
        None
    }
}

/// Skips leading ASCII whitespace bytes (space, tab, CR, LF).
fn skip_ascii_whitespace(bytes: &[u8]) -> &[u8] {
    let skip = bytes
        .iter()
        .take_while(|&&b| b == b' ' || b == b'\t' || b == b'\r' || b == b'\n')
        .count();
    &bytes[skip..]
}

/// Returns `true` if the label is a recognized alias for UTF-8.
fn is_utf8_label(label: &str) -> bool {
    matches!(label, "UTF-8" | "UTF8")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_utf8_bom() {
        let bytes = b"\xEF\xBB\xBF<?xml version=\"1.0\"?><root/>";
        let (encoding, skip) = detect_encoding(bytes);
        assert_eq!(encoding, "UTF-8");
        assert_eq!(skip, 3);
    }

    #[test]
    fn test_detect_utf16le_bom() {
        let bytes = b"\xFF\xFE<\x00r\x00o\x00o\x00t\x00";
        let (encoding, skip) = detect_encoding(bytes);
        assert_eq!(encoding, "UTF-16LE");
        assert_eq!(skip, 2);
    }

    #[test]
    fn test_detect_utf16be_bom() {
        let bytes = b"\xFE\xFF\x00<\x00r\x00o\x00o\x00t";
        let (encoding, skip) = detect_encoding(bytes);
        assert_eq!(encoding, "UTF-16BE");
        assert_eq!(skip, 2);
    }

    #[test]
    fn test_detect_no_bom() {
        let bytes = b"<?xml version=\"1.0\"?><root/>";
        let (encoding, skip) = detect_encoding(bytes);
        assert_eq!(encoding, "UTF-8");
        assert_eq!(skip, 0);
    }

    #[test]
    fn test_detect_empty_input() {
        let (encoding, skip) = detect_encoding(b"");
        assert_eq!(encoding, "UTF-8");
        assert_eq!(skip, 0);
    }

    #[test]
    fn test_detect_single_byte() {
        let (encoding, skip) = detect_encoding(b"\xEF");
        assert_eq!(encoding, "UTF-8");
        assert_eq!(skip, 0);
    }

    #[test]
    fn test_decode_utf8() {
        let bytes = b"<?xml version=\"1.0\"?><root>hello</root>";
        let result = decode_to_utf8(bytes).unwrap();
        assert_eq!(result, "<?xml version=\"1.0\"?><root>hello</root>");
    }

    #[test]
    fn test_decode_utf8_with_bom() {
        let bytes = b"\xEF\xBB\xBF<?xml version=\"1.0\"?><root/>";
        let result = decode_to_utf8(bytes).unwrap();
        assert_eq!(result, "<?xml version=\"1.0\"?><root/>");
    }

    #[test]
    fn test_decode_latin1() {
        // ISO-8859-1 encoded XML with encoding declaration.
        // The byte 0xE9 is 'e' with acute accent in ISO-8859-1.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>");
        bytes.extend_from_slice(b"<root>caf\xE9</root>");

        let result = decode_to_utf8(&bytes).unwrap();
        assert!(result.contains("caf\u{00E9}"));
        assert!(result.contains("<root>"));
    }

    #[test]
    fn test_transcode_utf8() {
        let result = transcode(b"hello world", "UTF-8").unwrap();
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_transcode_latin1() {
        // 0xE9 = 'e with acute' in ISO-8859-1
        let result = transcode(b"caf\xE9", "ISO-8859-1").unwrap();
        assert_eq!(result, "caf\u{00E9}");
    }

    #[test]
    fn test_transcode_unknown_encoding() {
        let result = transcode(b"hello", "UNKNOWN-ENCODING-42");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("unsupported encoding"));
    }

    #[test]
    fn test_extract_xml_decl_encoding_present() {
        let text = "<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><root/>";
        let enc = extract_xml_decl_encoding(text);
        assert_eq!(enc, Some("ISO-8859-1".to_string()));
    }

    #[test]
    fn test_extract_xml_decl_encoding_single_quotes() {
        let text = "<?xml version='1.0' encoding='UTF-8'?><root/>";
        let enc = extract_xml_decl_encoding(text);
        assert_eq!(enc, Some("UTF-8".to_string()));
    }

    #[test]
    fn test_extract_xml_decl_encoding_absent() {
        let text = "<?xml version=\"1.0\"?><root/>";
        let enc = extract_xml_decl_encoding(text);
        assert_eq!(enc, None);
    }

    #[test]
    fn test_extract_xml_decl_no_declaration() {
        let text = "<root/>";
        let enc = extract_xml_decl_encoding(text);
        assert_eq!(enc, None);
    }

    #[test]
    fn test_encoding_error_display() {
        let err = EncodingError::new("test error");
        assert_eq!(err.to_string(), "encoding error: test error");
    }

    #[test]
    fn test_encoding_error_is_error_trait() {
        let err = EncodingError::new("test");
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn test_decode_invalid_utf8() {
        // 0xFF 0xFE at the start is a UTF-16LE BOM, so use a different invalid sequence.
        // Bytes that are invalid UTF-8 without matching any BOM pattern.
        let bytes: &[u8] = &[0x80, 0x81, 0x82];
        let result = decode_to_utf8(bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_bytes_utf8() {
        use crate::tree::Document;

        let result = Document::parse_bytes(b"<root/>");
        assert!(result.is_ok());
        let doc = result.unwrap();
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_name(root), Some("root"));
    }

    #[test]
    fn test_parse_bytes_utf8_with_bom() {
        use crate::tree::Document;

        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"<root/>");
        let doc = Document::parse_bytes(&bytes).unwrap();
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_name(root), Some("root"));
    }
}
