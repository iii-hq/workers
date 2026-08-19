//! Error type for parsing and translation.

use alloc::string::String;
use core::fmt;

/// Failure from parsing a selector or translating it to XPath.
///
/// `Syntax` comes from the parser when the grammar is wrong. `Expression`
/// comes from the translator when a selector is valid but cannot be expressed
/// in XPath 1.0 or names an unknown pseudo-class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorError {
    /// A parse-time grammar error. The message matches the parser output.
    Syntax(String),
    /// A translate-time error. The message matches the translator output.
    Expression(String),
}

impl fmt::Display for SelectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SelectorError::Syntax(msg) => f.write_str(msg),
            SelectorError::Expression(msg) => f.write_str(msg),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for SelectorError {}
