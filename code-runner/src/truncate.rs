//! Output truncation: cap result and stream payloads without breaking UTF-8.
//!
//! Oversized responses are replaced or elided with explicit markers naming
//! what was dropped and how big it was, mirroring the engine's
//! `include_payloads` omission and context-manager's cap marker. A capped
//! response is still `success: true` — the run succeeded; only the echo
//! is bounded.

use serde_json::Value;

/// Cap a result value, replacing oversized payloads with a marker string.
///
/// If the serialized value fits in `max_bytes` or `max_bytes` is 0 (disabled),
/// returns the value unchanged. Otherwise, replaces it with a string marker
/// naming what was dropped and how to recover it: slicing, aggregation,
/// or write to `iii.files` and return a summary.
///
/// # Marker format
///
/// `<omitted: result was ~{KB} KB ({shape}); returning it whole would flood
/// the model context — print a slice, aggregate in code, or write it to
/// iii.files and return a summary>`
///
/// where `{KB}` is the original serialized size in kilobytes (bytes / 1024,
/// rounded down), and `{shape}` describes the structure:
/// - `N array elements` for arrays
/// - `N object keys` for objects
/// - `a N-char string` for strings (counting Unicode code points, not bytes)
/// - `a value` for other types
pub fn cap_result(value: Value, max_bytes: usize) -> Value {
    if max_bytes == 0 {
        return value;
    }

    let serialized = serde_json::to_string(&value).unwrap_or_default();
    if serialized.len() <= max_bytes {
        return value;
    }

    let kb = serialized.len() / 1024;
    let shape = match &value {
        Value::Array(a) => format!("{} array elements", a.len()),
        Value::Object(o) => format!("{} object keys", o.len()),
        Value::String(s) => format!("a {}-char string", s.chars().count()),
        _ => "a value".into(),
    };

    let marker = format!(
        "<omitted: result was ~{} KB ({}); returning it whole would flood the model context — \
         print a slice, aggregate in code, or write it to iii.files and return a summary>",
        kb, shape
    );

    Value::String(marker)
}

/// Cap a stream, keeping head and tail around a truncation marker.
///
/// If the string fits in `max_bytes` or `max_bytes` is 0 (disabled),
/// returns the string unchanged. Otherwise, keeps 60% of the available budget
/// for the head (floored to a UTF-8 character boundary) and 40% for the tail
/// (ceiled to a character boundary from the end), splicing a marker between.
///
/// The marker names the stream and original size: `\n[…{kind} truncated: was ~{KB} KB; middle omitted]\n`
/// where `{KB}` is the original size in kilobytes (bytes / 1024, rounded down).
///
/// This preserves both the first output (which often contains the first failure)
/// and the final summary line, while ensuring the total never exceeds
/// `max_bytes + marker allowance`.
pub fn cap_stream(kind: &str, s: String, max_bytes: usize) -> String {
    if max_bytes == 0 || s.len() <= max_bytes {
        return s;
    }

    let kb = s.len() / 1024;
    let marker = format!("\n[…{} truncated: was ~{} KB; middle omitted]\n", kind, kb);
    let marker_len = marker.len();

    // Budget for head and tail combined
    let budget = max_bytes.saturating_sub(marker_len);

    // Head = 60% of budget, tail = 40%
    let head_budget = (budget * 60) / 100;
    let tail_budget = budget - head_budget;

    // Extract head: floor to char boundary
    let head = floor_char_boundary(&s, head_budget);

    // Extract tail: ceil to char boundary from the end
    let tail = ceil_char_boundary_from_end(&s, tail_budget);

    format!("{}{}{}", head, marker, tail)
}

/// Find the floor of a character boundary at or before the given byte position.
///
/// If `pos` is already at a character boundary, returns `pos`.
/// Otherwise, scans backward to find the last character boundary.
fn floor_char_boundary(s: &str, pos: usize) -> &str {
    let pos = pos.min(s.len());
    let mut pos = pos;
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    &s[..pos]
}

/// Find the ceil of a character boundary, taking the last `size` bytes of the string.
///
/// Returns the rightmost `size` bytes (or fewer if fewer bytes are available),
/// floored to a character boundary. This extracts the tail of a string.
fn ceil_char_boundary_from_end(s: &str, size: usize) -> &str {
    if size >= s.len() {
        return s;
    }
    let start = s.len() - size;
    let mut start = start;
    // Scan forward from `start` to find the first valid character boundary
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_values_pass_through_untouched() {
        let v = serde_json::json!([1, 2, 3]);
        assert_eq!(cap_result(v.clone(), 32_768), v);
        assert_eq!(cap_stream("stdout", "hi\n".into(), 16_384), "hi\n");
    }

    #[test]
    fn zero_disables_the_cap() {
        let big = serde_json::json!(vec![0; 100_000]);
        assert_eq!(cap_result(big.clone(), 0), big);
    }

    #[test]
    fn oversized_result_becomes_a_marker_string_naming_shape_and_size() {
        let big = serde_json::json!((0..100_000).collect::<Vec<u32>>());
        let capped = cap_result(big, 32_768);
        let s = capped.as_str().expect("marker is a string");
        assert!(s.starts_with("<omitted: result was ~"));
        assert!(s.contains("100000 array elements"));
        assert!(s.contains("iii.files"));
    }

    #[test]
    fn oversized_stream_keeps_head_and_tail_around_the_marker() {
        let s: String = "x".repeat(40_000);
        let capped = cap_stream("stdout", s, 16_384);
        assert!(capped.len() <= 16_384 + 80, "marker allowance only");
        assert!(capped.contains("[…stdout truncated: was ~39 KB; middle omitted]"));
    }

    #[test]
    fn stream_truncation_respects_utf8_boundaries() {
        let s: String = "é".repeat(20_000); // 2-byte chars
        let capped = cap_stream("stdout", s, 16_384);
        assert!(capped.is_char_boundary(0) && std::str::from_utf8(capped.as_bytes()).is_ok());
    }
}
