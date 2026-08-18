//! Serde XML (de)serialization.
//!
//! This module provides `from_str` / `to_string` functions for converting between
//! XML text and Rust types via serde. It requires the `serde` feature.
//!
//! # Conventions
//!
//! - Element children map to struct fields by tag name
//! - Attributes are accessed via the `$attr` prefix: `#[serde(rename = "$attr:class")]`
//! - Text content is accessed via `$text`: `#[serde(rename = "$text")]`
//! - Sequences (repeated elements) are collected into `Vec<T>`
//! - The root element name is used as the struct name (or overridden via `#[serde(rename)]`)
//!
//! # Examples
//!
//! ```
//! # #[cfg(feature = "serde")]
//! # {
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Debug, Deserialize, Serialize, PartialEq)]
//! #[serde(rename = "book")]
//! struct Book {
//!     #[serde(rename = "$attr:isbn")]
//!     isbn: String,
//!     title: String,
//!     author: String,
//! }
//!
//! let xml = r#"<book isbn="978-0"><title>Rust</title><author>Alice</author></book>"#;
//! let book: Book = xmloxide::serde_xml::from_str(xml).unwrap();
//! assert_eq!(book.isbn, "978-0");
//! assert_eq!(book.title, "Rust");
//!
//! let xml_out = xmloxide::serde_xml::to_string(&book).unwrap();
//! assert!(xml_out.contains("<title>Rust</title>"));
//! # }
//! ```

mod de;
mod error;
mod ser;

pub use de::from_str;
pub use error::Error;
pub use ser::to_string;
