// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkerOpError {
    #[error("config I/O failed at {path:?}: {source}")]
    ConfigIo {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("config parse failed at {path:?}: {message}")]
    ConfigParse { path: PathBuf, message: String },

    #[error(
        "bundle manifest rejected: field {field:?} is not allowed for bundle workers: {reason}"
    )]
    BundleManifestRejected { field: String, reason: String },
}
