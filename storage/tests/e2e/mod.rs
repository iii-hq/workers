//! Opt-in end-to-end tests. Most are gated on env vars and skip
//! cleanly when unset. The `local` variant runs in CI by default
//! when a rustfs binary is discoverable.

pub mod gcs;
pub mod local;
pub mod r2;
pub mod s3;
