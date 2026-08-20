//! XML and HTML serialization.
//!
//! This module serializes a `Document` tree back to XML (or HTML) text.
//! The serializer handles proper escaping, XML declarations, and
//! formatting options. Includes Canonical XML (C14N) serialization for
//! producing deterministic byte sequences required by XML digital signatures.

pub mod c14n;
pub mod html;
pub mod xml;

pub use xml::{serialize, serialize_with_options, SerializeOptions};
