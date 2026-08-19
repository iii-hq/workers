//! String interning dictionary.
//!
//! The `Dict` provides string interning so that element names, attribute names,
//! namespace URIs, and other frequently repeated strings are stored once and
//! compared by index rather than by value. This is critical for parser
//! performance — hot-path string comparisons use `SymbolId` equality (a single
//! `u32` compare) instead of full string comparison.
//!
//! See libxml2's `dict.c` for the reference implementation.

use std::collections::HashMap;
use std::num::NonZeroU32;

/// An interned string identifier.
///
/// Two `SymbolId` values are equal if and only if they refer to the same
/// interned string within the same `Dict`. Comparing `SymbolId` is O(1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct SymbolId(NonZeroU32);

impl SymbolId {
    /// Returns the raw index value.
    #[must_use]
    pub fn as_u32(self) -> u32 {
        self.0.get()
    }
}

/// A string interning dictionary.
///
/// Stores unique strings and returns `SymbolId` handles for O(1) equality
/// comparisons. The dictionary owns all interned strings.
///
/// # Examples
///
/// ```ignore
/// use xmloxide::util::dict::Dict;
///
/// let mut dict = Dict::new();
/// let a = dict.intern("hello");
/// let b = dict.intern("hello");
/// let c = dict.intern("world");
///
/// assert_eq!(a, b);
/// assert_ne!(a, c);
/// assert_eq!(dict.resolve(a), "hello");
/// ```
#[derive(Debug)]
pub struct Dict {
    /// Map from string to its symbol id for O(1) lookup-or-insert.
    map: HashMap<String, SymbolId>,
    /// Indexed storage: symbol id → string. Index 0 is unused (`NonZeroU32`).
    strings: Vec<String>,
}

impl Dict {
    /// Creates a new empty dictionary.
    #[must_use]
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            // Index 0 is a placeholder since SymbolId uses NonZeroU32.
            strings: vec![String::new()],
        }
    }

    /// Interns a string and returns its `SymbolId`.
    ///
    /// If the string has been interned before, the existing `SymbolId` is
    /// returned. Otherwise, the string is stored and a new `SymbolId` is
    /// created.
    #[allow(clippy::cast_possible_truncation, clippy::expect_used)]
    pub fn intern(&mut self, s: &str) -> SymbolId {
        if let Some(&id) = self.map.get(s) {
            return id;
        }
        // strings.len() starts at 1 and only grows, so index >= 1 and
        // NonZeroU32::new will never return None. Truncation is acceptable
        // because we will never intern more than u32::MAX strings.
        let index = self.strings.len() as u32;
        let id = SymbolId(NonZeroU32::new(index).expect("symbol index overflow"));
        self.strings.push(s.to_owned());
        self.map.insert(s.to_owned(), id);
        id
    }

    /// Resolves a `SymbolId` back to its string.
    ///
    /// # Panics
    ///
    /// Panics if the `SymbolId` was not created by this dictionary.
    #[must_use]
    pub fn resolve(&self, id: SymbolId) -> &str {
        &self.strings[id.0.get() as usize]
    }

    /// Returns the number of interned strings.
    #[must_use]
    pub fn len(&self) -> usize {
        self.strings.len() - 1 // subtract the placeholder
    }

    /// Returns `true` if the dictionary contains no interned strings.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for Dict {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intern_returns_same_id_for_same_string() {
        let mut dict = Dict::new();
        let a = dict.intern("hello");
        let b = dict.intern("hello");
        assert_eq!(a, b);
    }

    #[test]
    fn test_intern_returns_different_ids_for_different_strings() {
        let mut dict = Dict::new();
        let a = dict.intern("hello");
        let b = dict.intern("world");
        assert_ne!(a, b);
    }

    #[test]
    fn test_resolve_returns_original_string() {
        let mut dict = Dict::new();
        let id = dict.intern("test string");
        assert_eq!(dict.resolve(id), "test string");
    }

    #[test]
    fn test_len_and_is_empty() {
        let mut dict = Dict::new();
        assert!(dict.is_empty());
        assert_eq!(dict.len(), 0);

        dict.intern("a");
        assert!(!dict.is_empty());
        assert_eq!(dict.len(), 1);

        dict.intern("b");
        assert_eq!(dict.len(), 2);

        // Interning duplicate doesn't increase length
        dict.intern("a");
        assert_eq!(dict.len(), 2);
    }

    #[test]
    fn test_empty_string_interning() {
        let mut dict = Dict::new();
        let id = dict.intern("");
        assert_eq!(dict.resolve(id), "");
    }

    #[test]
    fn test_symbol_id_as_u32() {
        let mut dict = Dict::new();
        let id = dict.intern("first");
        assert_eq!(id.as_u32(), 1); // First real entry is index 1
    }
}
