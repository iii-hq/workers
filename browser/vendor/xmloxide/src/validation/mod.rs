//! Document validation framework.
//!
//! This module provides schema validation for XML documents, supporting
//! DTD, `RelaxNG`, XML Schema (XSD), and ISO Schematron. Each validator
//! parses its schema format and checks document conformance, returning a
//! `ValidationResult` with errors and warnings.
//!
//! # Architecture
//!
//! The validation module is organized into:
//! - Common types (`ValidationResult`, `ValidationError`) used across all validators
//! - DTD validation (`dtd` submodule) for XML 1.0 DTD processing
//! - `RelaxNG` validation (`relaxng` submodule) for `RelaxNG` schema validation
//! - XML Schema validation (`xsd` submodule) for XSD 1.0 validation
//! - Schematron validation (`schematron` submodule) for ISO Schematron rule-based validation

pub mod dtd;
pub mod relaxng;
pub mod schematron;
pub mod xsd;

use std::fmt;

/// Result of validating a document against a schema (DTD, `RelaxNG`, XSD, etc.).
///
/// Contains the overall validity status plus any errors and warnings
/// encountered during validation.
///
/// # Examples
///
/// ```
/// use xmloxide::validation::ValidationResult;
///
/// let result = ValidationResult {
///     is_valid: true,
///     errors: vec![],
///     warnings: vec![],
/// };
/// assert!(result.is_valid);
/// ```
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Whether the document is valid according to the schema.
    pub is_valid: bool,
    /// Validation errors (each one makes the document invalid).
    pub errors: Vec<ValidationError>,
    /// Validation warnings (informational, do not affect validity).
    pub warnings: Vec<ValidationError>,
}

/// A validation error or warning with optional source location.
///
/// Carries a human-readable message and optional line/column information
/// for pinpointing the issue in the source document.
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// Human-readable description of the validation issue.
    pub message: String,
    /// The 1-based line number where the issue was detected, if known.
    pub line: Option<usize>,
    /// The 1-based column number where the issue was detected, if known.
    pub column: Option<usize>,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.line, self.column) {
            (Some(line), Some(col)) => write!(f, "{}:{}: {}", line, col, self.message),
            (Some(line), None) => write!(f, "line {}: {}", line, self.message),
            _ => write!(f, "{}", self.message),
        }
    }
}

impl fmt::Display for ValidationResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_valid {
            write!(f, "valid")?;
        } else {
            write!(f, "invalid ({} error(s))", self.errors.len())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_error_display_with_location() {
        let err = ValidationError {
            message: "missing required attribute".to_string(),
            line: Some(5),
            column: Some(10),
        };
        assert_eq!(err.to_string(), "5:10: missing required attribute");
    }

    #[test]
    fn test_validation_error_display_line_only() {
        let err = ValidationError {
            message: "unexpected element".to_string(),
            line: Some(3),
            column: None,
        };
        assert_eq!(err.to_string(), "line 3: unexpected element");
    }

    #[test]
    fn test_validation_error_display_no_location() {
        let err = ValidationError {
            message: "duplicate ID".to_string(),
            line: None,
            column: None,
        };
        assert_eq!(err.to_string(), "duplicate ID");
    }

    #[test]
    fn test_validation_result_display() {
        let valid = ValidationResult {
            is_valid: true,
            errors: vec![],
            warnings: vec![],
        };
        assert_eq!(valid.to_string(), "valid");

        let invalid = ValidationResult {
            is_valid: false,
            errors: vec![
                ValidationError {
                    message: "error 1".to_string(),
                    line: None,
                    column: None,
                },
                ValidationError {
                    message: "error 2".to_string(),
                    line: None,
                    column: None,
                },
            ],
            warnings: vec![],
        };
        assert_eq!(invalid.to_string(), "invalid (2 error(s))");
    }
}
