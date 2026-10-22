// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.

//! Typed worker-op errors and option types. The bundle path's
//! `validate_bundle_manifest` and `parse_bundle_resources` return
//! [`WorkerOpError`].

pub mod error;
pub use error::{WorkerOpError, WorkerOpErrorKind};

pub mod types;
pub use types::*;
