//! Worker templates behind `coder::list-templates` and
//! `coder::scaffold-worker`.

pub mod manifest;
pub mod source;

pub use manifest::{
    Language, compose_entry, load_worker_templates, replace_token, validate_worker_name,
};
pub use source::{ResolvedSource, TemplateSourceInfo, resolve_source};
