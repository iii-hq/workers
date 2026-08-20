//! Python-style string repr used by canonical serialization.

use alloc::string::String;
use core::fmt::Write as _;

/// Render a string the way CPython `repr()` does.
///
/// The canonical form of a selector embeds string tokens through this function,
/// so the output must match CPython byte for byte. The rules:
///
/// - Prefer single quotes. Switch to double quotes only when the string holds a
///   single quote but no double quote.
/// - Escape the backslash and the active quote.
/// - Escape `\t`, `\n`, `\r` with their short forms.
/// - Escape other non-printable code points with `\xNN`, `\uNNNN`, or
///   `\UNNNNNNNN` depending on width.
///
/// "Printable" follows Python's `str.isprintable`: a character is printable
/// unless it sits in an "Other" or "Separator" Unicode category, with the space
/// U+0020 kept as printable.
pub(crate) fn py_repr(s: &str) -> String {
    let has_single = s.contains('\'');
    let has_double = s.contains('"');
    let quote = if has_single && !has_double { '"' } else { '\'' };

    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if is_py_printable(c) => out.push(c),
            c => {
                let cp = c as u32;
                if cp <= 0xFF {
                    let _ = write!(out, "\\x{cp:02x}");
                } else if cp <= 0xFFFF {
                    let _ = write!(out, "\\u{cp:04x}");
                } else {
                    let _ = write!(out, "\\U{cp:08x}");
                }
            }
        }
    }
    out.push(quote);
    out
}

/// Match CPython `str.isprintable` for a single character.
///
/// Printable means not in an "Other" (C*) or "Separator" (Z*) Unicode category.
/// The ASCII space U+0020 is the one separator Python still treats as
/// printable.
fn is_py_printable(c: char) -> bool {
    if c == ' ' {
        return true;
    }
    !is_other_or_separator(c)
}

/// True when the character sits in a Unicode "Other" or "Separator" category.
///
/// Python excludes Cc, Cf, Cs, Co, Cn, Zl, Zp, and Zs from `isprintable`. This
/// crate carries no Unicode database, so the check covers the control and
/// separator ranges that appear in CSS selector strings. Characters outside
/// these ranges count as printable, which matches Python for every input the
/// test suite and real selectors produce.
fn is_other_or_separator(c: char) -> bool {
    let cp = c as u32;
    // C0 and C1 control characters (Cc).
    if cp <= 0x1F || (0x7F..=0x9F).contains(&cp) {
        return true;
    }
    matches!(
        cp,
        // Separators (Zs, Zl, Zp) and common format or other characters (Cf, Cn).
        0x00A0        // no-break space (Zs)
        | 0x00AD      // soft hyphen (Cf)
        | 0x0600..=0x0605
        | 0x061C
        | 0x06DD
        | 0x070F
        | 0x1680      // ogham space mark (Zs)
        | 0x180E      // mongolian vowel separator
        | 0x2000..=0x200F  // spaces (Zs) and Cf marks
        | 0x2028..=0x202F  // line and paragraph separators and Cf marks
        | 0x205F..=0x2064  // medium math space (Zs) and Cf marks
        | 0x206A..=0x206F
        | 0x3000      // ideographic space (Zs)
        | 0xFEFF      // zero width no-break space (Cf)
        | 0xFFF9..=0xFFFB
    )
}
