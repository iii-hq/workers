//! Error types and diagnostics for XML parsing.
//!
//! This module provides structured error reporting with source location tracking,
//! matching libxml2's error reporting model. Errors carry line, column, and byte
//! offset information for precise diagnostics.
//!
//! The parser supports **error recovery mode**: it collects errors into a
//! `Vec<ParseDiagnostic>` while still producing a (possibly partial) tree.

use std::fmt;

/// Severity level for a parse diagnostic, matching libxml2's `xmlErrorLevel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorSeverity {
    /// A non-fatal issue that doesn't prevent parsing.
    Warning,
    /// A recoverable error — the parser can continue but the document is malformed.
    Error,
    /// An unrecoverable error — parsing must stop.
    Fatal,
}

impl fmt::Display for ErrorSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Warning => write!(f, "warning"),
            Self::Error => write!(f, "error"),
            Self::Fatal => write!(f, "fatal error"),
        }
    }
}

/// Source location within an XML document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SourceLocation {
    /// 1-based line number.
    pub line: u32,
    /// 1-based column number (in characters, not bytes).
    pub column: u32,
    /// 0-based byte offset from the start of the input.
    pub byte_offset: usize,
}

impl fmt::Display for SourceLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// A single diagnostic emitted during parsing.
///
/// Diagnostics are collected when the parser operates in recovery mode,
/// allowing it to produce a partial tree even when the input is malformed.
#[derive(Debug, Clone)]
pub struct ParseDiagnostic {
    /// The severity of this diagnostic.
    pub severity: ErrorSeverity,
    /// Human-readable error message.
    pub message: String,
    /// Where in the source this error occurred.
    pub location: SourceLocation,
}

impl fmt::Display for ParseDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} at {}",
            self.severity, self.message, self.location
        )
    }
}

/// The error type returned when XML parsing fails.
#[derive(Debug, Clone)]
pub struct ParseError {
    /// The primary error message.
    pub message: String,
    /// Where in the source the fatal error occurred.
    pub location: SourceLocation,
    /// All diagnostics collected before the fatal error (in recovery mode,
    /// this includes warnings and recovered errors).
    pub diagnostics: Vec<ParseDiagnostic>,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "parse error at {}: {}", self.location, self.message)
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_location_display() {
        let loc = SourceLocation {
            line: 10,
            column: 5,
            byte_offset: 42,
        };
        assert_eq!(loc.to_string(), "10:5");
    }

    #[test]
    fn test_parse_error_display() {
        let err = ParseError {
            message: "unexpected end of input".to_string(),
            location: SourceLocation {
                line: 1,
                column: 15,
                byte_offset: 14,
            },
            diagnostics: vec![],
        };
        assert_eq!(
            err.to_string(),
            "parse error at 1:15: unexpected end of input"
        );
    }

    #[test]
    fn test_parse_diagnostic_display() {
        let diag = ParseDiagnostic {
            severity: ErrorSeverity::Warning,
            message: "attribute value not quoted".to_string(),
            location: SourceLocation {
                line: 3,
                column: 10,
                byte_offset: 50,
            },
        };
        assert_eq!(
            diag.to_string(),
            "warning: attribute value not quoted at 3:10"
        );
    }

    #[test]
    fn test_error_severity_display() {
        assert_eq!(ErrorSeverity::Warning.to_string(), "warning");
        assert_eq!(ErrorSeverity::Error.to_string(), "error");
        assert_eq!(ErrorSeverity::Fatal.to_string(), "fatal error");
    }

    #[test]
    fn test_parse_error_is_error_trait() {
        let err = ParseError {
            message: "test".to_string(),
            location: SourceLocation::default(),
            diagnostics: vec![],
        };
        // Verify it implements std::error::Error
        let _: &dyn std::error::Error = &err;
    }
}
