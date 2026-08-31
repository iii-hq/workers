//! One module per `browser::*` parse op, plus `common` for the
//! response-shaping helpers they share. Each op is a
//! `fn(&Value) -> Result<Value, String>` named `op`; `super::op_for` maps a
//! function id to one of them.

pub mod common;
pub mod css_fn;
pub mod describe;
pub mod extract;
pub mod find;
pub mod find_by_regex;
pub mod find_by_text;
pub mod find_similar;
pub mod regex_fn;
pub mod to_markdown;
pub mod xpath_fn;
