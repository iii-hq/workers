// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.

//! Offline whole-generation storage. Private operations have no runtime issuer.
//! worker lifecycle calls only the read-only admission inspector; hashes/JSON
//! and old tokens never manufacture authority. See README.md for the exact seam.

pub(crate) mod admission;
mod fs;
mod store;
mod tree;

use serde::{Deserialize, Serialize};
use std::{fs::File, io};
use thiserror::Error;

const FORMAT: u32 = 1;
const MAX_RECORD: usize = 64 * 1024;
const MAX_ENTRIES: usize = 16_384;
const MAX_XATTR_BYTES: usize = 1024 * 1024;

#[derive(Debug, Error)]
enum Error {
    #[error("unsafe custody or source identity changed")]
    Custody,
    #[error("unsupported filesystem feature or entry type: {0}")]
    Unsupported(&'static str),
    #[error("invalid or corrupt generation record")]
    Corrupt,
    #[error("generation compare-and-swap conflict")]
    Conflict,
    #[error("operation already staged or name occupied")]
    Collision,
    #[error("incomplete operation retained; no generation selected")]
    Interrupted,
    #[error("selector publication may have committed; reread and recover before retry")]
    Uncertain,
    #[error("local generation I/O: {0}")]
    Io(#[from] io::Error),
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Layout {
    OverlayImage,
    LegacyClone,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Provision {
    Path,
    Bundle,
}

/// Identity/integrity assertions already proven by the future local coordinator.
/// Digests alone never establish original boot continuity or token retirement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Environment {
    source_version: String,
    layout: Layout,
    provision: Provision,
    adapter_sha256: String,
    engine_sha256: String,
    kernel_sha256: String,
    init_sha256: String,
    profile_sha256: String,
    app_sha256: String,
    sdk_lock_sha256: String,
    provisioning_sha256: String,
    lower_sha256: Option<String>,
}
impl Environment {
    fn validate(&self) -> Result<()> {
        if !matches!(self.source_version.as_str(), "0.3.0" | "0.3.1") {
            return Err(Error::Corrupt);
        }
        for digest in [
            &self.adapter_sha256,
            &self.engine_sha256,
            &self.kernel_sha256,
            &self.init_sha256,
            &self.profile_sha256,
            &self.app_sha256,
            &self.sdk_lock_sha256,
            &self.provisioning_sha256,
        ] {
            if !sha(digest) {
                return Err(Error::Corrupt);
            }
        }
        match (&self.layout, &self.lower_sha256) {
            (Layout::OverlayImage, Some(value)) if sha(value) => Ok(()),
            (Layout::LegacyClone, None) => Ok(()),
            _ => Err(Error::Corrupt),
        }
    }
}
fn sha(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

/// No Deserialize/public constructor/boolean grant. The future OS-peer
/// coordinator must retain its owner/project lease AND verified old writer and
/// credential retirement for this lifetime. Test fixtures are the only issuer.
struct OfflineGrant {
    binding: String,
    environment: Environment,
    source: fs::Dir,
    pinned: tree::Inventory,
    _source_lease: File,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    binding: String,
    operation: String,
    generation: u64,
    parent: u64,
    parent_operation: Option<String>,
    #[serde(deserialize_with = "object")]
    environment: Environment,
    backup: String,
    backup_digest: String,
    snapshot_digest: String,
}
impl Record {
    fn validate(&self, binding: &str) -> Result<()> {
        self.environment.validate()?;
        if self.version != FORMAT
            || self.binding != binding
            || !sha(binding)
            || !id(&self.operation)
            || self.generation == 0
            || self.parent.checked_add(1) != Some(self.generation)
            || self.backup != self.operation
            || !sha(&self.backup_digest)
            || !sha(&self.snapshot_digest)
        {
            return Err(Error::Corrupt);
        }
        if (self.parent == 0 && self.parent_operation.is_some())
            || (self.parent > 0 && !self.parent_operation.as_ref().is_some_and(|op| id(op)))
        {
            return Err(Error::Corrupt);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Selector {
    version: u32,
    binding: String,
    generation: u64,
    operation: String,
    record_sha256: String,
}

/// Steps are named at actual syscall boundaries. Fault injection exists only in
/// cfg(test); normal code still propagates EVERY fsync/rename/copy error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Copy,
    Metadata,
    FileSync,
    DirectorySync,
    BackupRename,
    BackupPublishedSync,
    GenerationRename,
    GenerationPublishedSync,
    GenerationRecordSync,
    GenerationDirectorySync,
    FilesystemSync,
    SelectorPreparedSync,
    RecoverySelectorSync,
    SelectorWrite,
    SelectorRename,
    SelectorSync,
}
#[derive(Default)]
struct Faults {
    #[cfg(test)]
    fail: Option<(Step, i32)>,
}
impl Faults {
    fn at(&mut self, step: Step) -> Result<()> {
        #[cfg(test)]
        if self.fail.is_some_and(|(at, _)| at == step) {
            let (_, errno) = self.fail.take().unwrap();
            return Err(io::Error::from_raw_os_error(errno).into());
        }
        let _ = step;
        Ok(())
    }
}
fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value).map_err(|_| Error::Corrupt)?;
    if bytes.len() > MAX_RECORD {
        return Err(Error::Corrupt);
    }
    Ok(bytes)
}
fn object<'de, D, T>(deserializer: D) -> std::result::Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Map<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Map<T> {
        type Value = T;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a generation object")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            map: M,
        ) -> std::result::Result<T, M::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    deserializer.deserialize_map(Map(std::marker::PhantomData))
}
fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    if bytes.is_empty() || bytes.len() > MAX_RECORD {
        return Err(Error::Corrupt);
    }
    // Preserve duplicate fields for typed Serde, and disallow positional structs.
    if bytes.iter().find(|b| !b.is_ascii_whitespace()) != Some(&b'{') {
        return Err(Error::Corrupt);
    }
    serde_json::from_slice(bytes).map_err(|_| Error::Corrupt)
}

#[cfg(test)]
mod tests;
