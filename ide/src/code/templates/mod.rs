//! Worker templates behind `coder::list-templates` and
//! `coder::scaffold-worker`.

pub mod manifest;

pub use manifest::{
    compose_entry, load_worker_templates, replace_token, validate_worker_name, Language,
    WorkerBlock, WorkerTemplate,
};
