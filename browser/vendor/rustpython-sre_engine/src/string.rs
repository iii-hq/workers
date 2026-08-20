use rustpython_wtf8::Wtf8;

#[derive(Debug, Clone, Copy)]
pub struct StringCursor {
    pub(crate) ptr: *const u8,
    pub position: usize,
}

impl Default for StringCursor {
    fn default() -> Self {
        Self {
            ptr: core::ptr::null(),
            position: 0,
        }
    }
}

pub trait StrDrive: Copy {
    fn count(&self) -> usize;
    fn create_cursor(&self, n: usize) -> StringCursor;
    fn adjust_cursor(&self, cursor: &mut StringCursor, n: usize);
    fn advance(cursor: &mut StringCursor) -> u32;
    fn peek(cursor: &StringCursor) -> u32;
    fn skip(cursor: &mut StringCursor, n: usize);
    fn back_advance(cursor: &mut StringCursor) -> u32;
    fn back_peek(cursor: &StringCursor) -> u32;
    fn back_skip(cursor: &mut StringCursor, n: usize);
}

impl StrDrive for &[u8] {
    #[inline]
    fn count(&self) -> usize {
        self.len()
    }

    #[inline]
    fn create_cursor(&self, n: usize) -> StringCursor {
        StringCursor {
            ptr: self[n..].as_ptr(),
            position: n,
        }
    }

    #[inline]
    fn adjust_cursor(&self, cursor: &mut StringCursor, n: usize) {
        cursor.position = n;
        cursor.ptr = self[n..].as_ptr();
    }

    #[inline]
    fn advance(cursor: &mut StringCursor) -> u32 {
        cursor.position += 1;
        unsafe { cursor.ptr = cursor.ptr.add(1) };
        unsafe { *cursor.ptr as u32 }
    }

    #[inline]
    fn peek(cursor: &StringCursor) -> u32 {
        unsafe { *cursor.ptr as u32 }
    }

    #[inline]
    fn skip(cursor: &mut StringCursor, n: usize) {
        cursor.position += n;
        unsafe { cursor.ptr = cursor.ptr.add(n) };
    }

    #[inline]
    fn back_advance(cursor: &mut StringCursor) -> u32 {
        cursor.position -= 1;
        unsafe { cursor.ptr = cursor.ptr.sub(1) };
        unsafe { *cursor.ptr as u32 }
    }

    #[inline]
    fn back_peek(cursor: &StringCursor) -> u32 {
        unsafe { *cursor.ptr.offset(-1) as u32 }
    }

    #[inline]
    fn back_skip(cursor: &mut StringCursor, n: usize) {
        cursor.position -= n;
        unsafe { cursor.ptr = cursor.ptr.sub(n) };
    }
}

impl StrDrive for &str {
    #[inline]
    fn count(&self) -> usize {
        self.chars().count()
    }

    #[inline]
    fn create_cursor(&self, n: usize) -> StringCursor {
        let mut cursor = StringCursor {
            ptr: self.as_ptr(),
            position: 0,
        };
        Self::skip(&mut cursor, n);
        cursor
    }

    #[inline]
    fn adjust_cursor(&self, cursor: &mut StringCursor, n: usize) {
        if cursor.ptr.is_null() || cursor.position > n {
            *cursor = Self::create_cursor(self, n);
        } else if cursor.position < n {
            Self::skip(cursor, n - cursor.position);
        }
    }

    #[inline]
    fn advance(cursor: &mut StringCursor) -> u32 {
        cursor.position += 1;
        unsafe { next_code_point(&mut cursor.ptr) }
    }

    #[inline]
    fn peek(cursor: &StringCursor) -> u32 {
        let mut ptr = cursor.ptr;
        unsafe { next_code_point(&mut ptr) }
    }

    #[inline]
    fn skip(cursor: &mut StringCursor, n: usize) {
        cursor.position += n;
        for _ in 0..n {
            unsafe { next_code_point(&mut cursor.ptr) };
        }
    }

    #[inline]
    fn back_advance(cursor: &mut StringCursor) -> u32 {
        cursor.position -= 1;
        unsafe { next_code_point_reverse(&mut cursor.ptr) }
    }

    #[inline]
    fn back_peek(cursor: &StringCursor) -> u32 {
        let mut ptr = cursor.ptr;
        unsafe { next_code_point_reverse(&mut ptr) }
    }

    #[inline]
    fn back_skip(cursor: &mut StringCursor, n: usize) {
        cursor.position -= n;
        for _ in 0..n {
            unsafe { next_code_point_reverse(&mut cursor.ptr) };
        }
    }
}

impl StrDrive for &Wtf8 {
    #[inline]
    fn count(&self) -> usize {
        self.code_points().count()
    }

    #[inline]
    fn create_cursor(&self, n: usize) -> StringCursor {
        let mut cursor = StringCursor {
            ptr: self.as_bytes().as_ptr(),
            position: 0,
        };
        Self::skip(&mut cursor, n);
        cursor
    }

    #[inline]
    fn adjust_cursor(&self, cursor: &mut StringCursor, n: usize) {
        if cursor.ptr.is_null() || cursor.position > n {
            *cursor = Self::create_cursor(self, n);
        } else if cursor.position < n {
            Self::skip(cursor, n - cursor.position);
        }
    }

    #[inline]
    fn advance(cursor: &mut StringCursor) -> u32 {
        cursor.position += 1;
        unsafe { next_code_point(&mut cursor.ptr) }
    }

    #[inline]
    fn peek(cursor: &StringCursor) -> u32 {
        let mut ptr = cursor.ptr;
        unsafe { next_code_point(&mut ptr) }
    }

    #[inline]
    fn skip(cursor: &mut StringCursor, n: usize) {
        cursor.position += n;
        for _ in 0..n {
            unsafe { next_code_point(&mut cursor.ptr) };
        }
    }

    #[inline]
    fn back_advance(cursor: &mut StringCursor) -> u32 {
        cursor.position -= 1;
        unsafe { next_code_point_reverse(&mut cursor.ptr) }
    }

    #[inline]
    fn back_peek(cursor: &StringCursor) -> u32 {
        let mut ptr = cursor.ptr;
        unsafe { next_code_point_reverse(&mut ptr) }
    }

    #[inline]
    fn back_skip(cursor: &mut StringCursor, n: usize) {
        cursor.position -= n;
        for _ in 0..n {
            unsafe { next_code_point_reverse(&mut cursor.ptr) };
        }
    }
}

/// Reads the next code point out of a byte iterator (assuming a
/// UTF-8-like encoding).
///
/// # Safety
///
/// `bytes` must produce a valid UTF-8-like (UTF-8 or WTF-8) string
#[inline]
const unsafe fn next_code_point(ptr: &mut *const u8) -> u32 {
    // Decode UTF-8
    let x = unsafe { **ptr };
    *ptr = unsafe { ptr.offset(1) };

    if x < 128 {
        return x as u32;
    }

    // Multibyte case follows
    // Decode from a byte combination out of: [[[x y] z] w]
    // NOTE: Performance is sensitive to the exact formulation here
    let init = utf8_first_byte(x, 2);
    // SAFETY: `bytes` produces an UTF-8-like string,
    // so the iterator must produce a value here.
    let y = unsafe { **ptr };
    *ptr = unsafe { ptr.offset(1) };
    let mut ch = utf8_acc_cont_byte(init, y);
    if x >= 0xE0 {
        // [[x y z] w] case
        // 5th bit in 0xE0 .. 0xEF is always clear, so `init` is still valid
        // SAFETY: `bytes` produces an UTF-8-like string,
        // so the iterator must produce a value here.
        let z = unsafe { **ptr };
        *ptr = unsafe { ptr.offset(1) };
        let y_z = utf8_acc_cont_byte((y & CONT_MASK) as u32, z);
        ch = (init << 12) | y_z;
        if x >= 0xF0 {
            // [x y z w] case
            // use only the lower 3 bits of `init`
            // SAFETY: `bytes` produces an UTF-8-like string,
            // so the iterator must produce a value here.
            let w = unsafe { **ptr };
            *ptr = unsafe { ptr.offset(1) };
            ch = ((init & 7) << 18) | utf8_acc_cont_byte(y_z, w);
        }
    }

    ch
}

/// Reads the last code point out of a byte iterator (assuming a
/// UTF-8-like encoding).
///
/// # Safety
///
/// `bytes` must produce a valid UTF-8-like (UTF-8 or WTF-8) string
#[inline]
const unsafe fn next_code_point_reverse(ptr: &mut *const u8) -> u32 {
    // Decode UTF-8
    *ptr = unsafe { ptr.offset(-1) };
    let w = match unsafe { **ptr } {
        next_byte if next_byte < 128 => return next_byte as u32,
        back_byte => back_byte,
    };

    // Multibyte case follows
    // Decode from a byte combination out of: [x [y [z w]]]
    let mut ch;
    // SAFETY: `bytes` produces an UTF-8-like string,
    // so the iterator must produce a value here.
    *ptr = unsafe { ptr.offset(-1) };
    let z = unsafe { **ptr };
    ch = utf8_first_byte(z, 2);
    if utf8_is_cont_byte(z) {
        // SAFETY: `bytes` produces an UTF-8-like string,
        // so the iterator must produce a value here.
        *ptr = unsafe { ptr.offset(-1) };
        let y = unsafe { **ptr };
        ch = utf8_first_byte(y, 3);
        if utf8_is_cont_byte(y) {
            // SAFETY: `bytes` produces an UTF-8-like string,
            // so the iterator must produce a value here.
            *ptr = unsafe { ptr.offset(-1) };
            let x = unsafe { **ptr };
            ch = utf8_first_byte(x, 4);
            ch = utf8_acc_cont_byte(ch, y);
        }
        ch = utf8_acc_cont_byte(ch, z);
    }
    ch = utf8_acc_cont_byte(ch, w);

    ch
}

/// Returns the initial codepoint accumulator for the first byte.
/// The first byte is special, only want bottom 5 bits for width 2, 4 bits
/// for width 3, and 3 bits for width 4.
#[inline]
const fn utf8_first_byte(byte: u8, width: u32) -> u32 {
    (byte & (0x7F >> width)) as u32
}

/// Returns the value of `ch` updated with continuation byte `byte`.
#[inline]
const fn utf8_acc_cont_byte(ch: u32, byte: u8) -> u32 {
    (ch << 6) | (byte & CONT_MASK) as u32
}

/// Checks whether the byte is a UTF-8 continuation byte (i.e., starts with the
/// bits `10`).
#[inline]
const fn utf8_is_cont_byte(byte: u8) -> bool {
    (byte as i8) < -64
}

/// Mask of the value bits of a continuation byte.
const CONT_MASK: u8 = 0b0011_1111;

const fn is_py_ascii_whitespace(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\x0C' | b'\r' | b' ' | b'\x0B')
}

#[inline]
pub(crate) fn is_word(ch: u32) -> bool {
    ch == '_' as u32
        || u8::try_from(ch)
            .map(|x| x.is_ascii_alphanumeric())
            .unwrap_or(false)
}
#[inline]
pub(crate) fn is_space(ch: u32) -> bool {
    u8::try_from(ch)
        .map(is_py_ascii_whitespace)
        .unwrap_or(false)
}
#[inline]
pub(crate) fn is_digit(ch: u32) -> bool {
    u8::try_from(ch)
        .map(|x| x.is_ascii_digit())
        .unwrap_or(false)
}
#[inline]
pub(crate) fn is_loc_alnum(ch: u32) -> bool {
    // FIXME: Ignore the locales
    u8::try_from(ch)
        .map(|x| x.is_ascii_alphanumeric())
        .unwrap_or(false)
}
#[inline]
pub(crate) fn is_loc_word(ch: u32) -> bool {
    ch == '_' as u32 || is_loc_alnum(ch)
}
#[inline]
pub(crate) const fn is_linebreak(ch: u32) -> bool {
    ch == '\n' as u32
}
#[inline]
pub fn lower_ascii(ch: u32) -> u32 {
    u8::try_from(ch)
        .map(|x| x.to_ascii_lowercase() as u32)
        .unwrap_or(ch)
}
#[inline]
pub(crate) fn lower_locate(ch: u32) -> u32 {
    // FIXME: Ignore the locales
    lower_ascii(ch)
}
#[inline]
pub(crate) fn upper_locate(ch: u32) -> u32 {
    // FIXME: Ignore the locales
    u8::try_from(ch)
        .map(|x| x.to_ascii_uppercase() as u32)
        .unwrap_or(ch)
}
#[inline]
pub(crate) fn is_uni_digit(ch: u32) -> bool {
    const DECIMAL_RANGES: &[(u32, u32)] = &[
        (0x0030, 0x0039),
        (0x0660, 0x0669),
        (0x06F0, 0x06F9),
        (0x07C0, 0x07C9),
        (0x0966, 0x096F),
        (0x09E6, 0x09EF),
        (0x0A66, 0x0A6F),
        (0x0AE6, 0x0AEF),
        (0x0B66, 0x0B6F),
        (0x0BE6, 0x0BEF),
        (0x0C66, 0x0C6F),
        (0x0CE6, 0x0CEF),
        (0x0D66, 0x0D6F),
        (0x0DE6, 0x0DEF),
        (0x0E50, 0x0E59),
        (0x0ED0, 0x0ED9),
        (0x0F20, 0x0F29),
        (0x1040, 0x1049),
        (0x1090, 0x1099),
        (0x17E0, 0x17E9),
        (0x1810, 0x1819),
        (0x1946, 0x194F),
        (0x19D0, 0x19D9),
        (0x1A80, 0x1A89),
        (0x1A90, 0x1A99),
        (0x1B50, 0x1B59),
        (0x1BB0, 0x1BB9),
        (0x1C40, 0x1C49),
        (0x1C50, 0x1C59),
        (0xA620, 0xA629),
        (0xA8D0, 0xA8D9),
        (0xA900, 0xA909),
        (0xA9D0, 0xA9D9),
        (0xA9F0, 0xA9F9),
        (0xAA50, 0xAA59),
        (0xABF0, 0xABF9),
        (0xFF10, 0xFF19),
        (0x104A0, 0x104A9),
        (0x10D30, 0x10D39),
        (0x11066, 0x1106F),
        (0x110F0, 0x110F9),
        (0x11136, 0x1113F),
        (0x111D0, 0x111D9),
        (0x112F0, 0x112F9),
        (0x11450, 0x11459),
        (0x114D0, 0x114D9),
        (0x11650, 0x11659),
        (0x116C0, 0x116C9),
        (0x11730, 0x11739),
        (0x118E0, 0x118E9),
        (0x11950, 0x11959),
        (0x11C50, 0x11C59),
        (0x11D50, 0x11D59),
        (0x11DA0, 0x11DA9),
        (0x11F50, 0x11F59),
        (0x16A60, 0x16A69),
        (0x16AC0, 0x16AC9),
        (0x16B50, 0x16B59),
        (0x1D7CE, 0x1D7FF),
        (0x1E140, 0x1E149),
        (0x1E2F0, 0x1E2F9),
        (0x1E4F0, 0x1E4F9),
        (0x1E950, 0x1E959),
        (0x1FBF0, 0x1FBF9),
    ];
    DECIMAL_RANGES
        .iter()
        .any(|&(start, end)| start <= ch && ch <= end)
}
#[inline]
pub(crate) fn is_uni_space(ch: u32) -> bool {
    // TODO: check with cpython
    is_space(ch)
        || matches!(
            ch,
            0x0009
                | 0x000A
                | 0x000B
                | 0x000C
                | 0x000D
                | 0x001C
                | 0x001D
                | 0x001E
                | 0x001F
                | 0x0020
                | 0x0085
                | 0x00A0
                | 0x1680
                | 0x2000
                | 0x2001
                | 0x2002
                | 0x2003
                | 0x2004
                | 0x2005
                | 0x2006
                | 0x2007
                | 0x2008
                | 0x2009
                | 0x200A
                | 0x2028
                | 0x2029
                | 0x202F
                | 0x205F
                | 0x3000
        )
}
#[inline]
pub(crate) const fn is_uni_linebreak(ch: u32) -> bool {
    matches!(
        ch,
        0x000A | 0x000B | 0x000C | 0x000D | 0x001C | 0x001D | 0x001E | 0x0085 | 0x2028 | 0x2029
    )
}
#[inline]
pub(crate) fn is_uni_alnum(ch: u32) -> bool {
    // TODO: check with cpython
    char::try_from(ch)
        .map(|x| x.is_alphanumeric())
        .unwrap_or(false)
}
#[inline]
pub(crate) fn is_uni_word(ch: u32) -> bool {
    ch == '_' as u32 || is_uni_alnum(ch)
}
#[inline]
pub fn lower_unicode(ch: u32) -> u32 {
    // TODO: check with cpython
    char::try_from(ch)
        .map(|x| x.to_lowercase().next().unwrap() as u32)
        .unwrap_or(ch)
}
#[inline]
pub fn upper_unicode(ch: u32) -> u32 {
    // TODO: check with cpython
    char::try_from(ch)
        .map(|x| x.to_uppercase().next().unwrap() as u32)
        .unwrap_or(ch)
}
