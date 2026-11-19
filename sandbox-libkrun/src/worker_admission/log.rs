// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! Exact ordinary log retention, not generic mutable-file authority. One ledger
//! slot per declared pair; no per-record stage/prior or journal allocation.
use super::{
    Lease, PathRole,
    catalog::{Hold, HoldKind, ResourceOwner},
    storage::*,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io,
    os::unix::fs::{FileExt, MetadataExt},
    path::Path,
    sync::{Arc, Mutex},
};
const MAX_LOGS: usize = 16;
const MAX_RECORD: usize = 1024 * 1024;
const MAX_CAP: u64 = 64 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum LogPhase {
    Clean,
    Pending,
    Failed,
    Finished,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LogCursor {
    pub base: u64,
    pub len: u64,
    pub previous_base: u64,
    pub previous_len: u64,
}
impl LogCursor {
    pub(crate) fn end(&self) -> u64 {
        self.base.saturating_add(self.len) // persisted values are overflow-validated
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LogPending {
    pub reset: bool,
    pub next: LogCursor,
    pub current_hash: String,
    pub previous_hash: Option<String>,
    pub chain: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LogRecord {
    pub id: String,
    pub holder: String,
    pub current: HostPath,
    pub previous: Option<HostPath>,
    pub current_id: Identity,
    pub previous_id: Option<Identity>,
    pub cap: u64,
    pub phase: LogPhase,
    pub cursor: LogCursor,
    pub sequence: u64,
    pub chain: String,
    pub pending: Option<LogPending>,
    pub current_hash: String,
    pub previous_hash: Option<String>,
    pub current_stamp: LogStamp,
    pub previous_stamp: Option<LogStamp>,
}
pub(crate) struct LogSpec<'a> {
    pub current: &'a Path,
    pub previous: Option<&'a Path>,
    pub cap_bytes: u64,
}
#[derive(Clone)]
pub(crate) struct LogWriter(Arc<LogHeld>);
struct LogHeld {
    lease: Lease,
    id: String,
    parent: Directory,
    current: File,
    previous: Option<File>,
    state: Mutex<Local>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LogStamp {
    size: u64,
    mtime: i64,
    mtime_ns: i64,
    ctime: i64,
    ctime_ns: i64,
}
fn log_stamp(file: &File) -> io::Result<LogStamp> {
    let m = file.metadata()?;
    Ok(LogStamp {
        size: m.len(),
        mtime: m.mtime(),
        mtime_ns: m.mtime_nsec(),
        ctime: m.ctime(),
        ctime_ns: m.ctime_nsec(),
    })
}
struct Local {
    hasher: Sha256,
    failed: bool,
    finished: bool,
    #[cfg(test)]
    fault: Option<&'static str>,
}
pub(crate) struct LogRead {
    pub bytes: Vec<u8>,
    pub next: u64,
    pub end: u64,
    pub oldest: u64,
}
fn hash_state(file: &File, size: u64) -> io::Result<Sha256> {
    let mut h = Sha256::new();
    let mut bytes = [0; 65536];
    let mut at = 0;
    while at < size {
        let n = (size - at).min(bytes.len() as u64) as usize;
        file.read_exact_at(&mut bytes[..n], at)?;
        h.update(&bytes[..n]);
        at += n as u64;
    }
    Ok(h)
}
fn hash(file: &File, size: u64, tail: &[u8]) -> io::Result<String> {
    let mut h = Sha256::new();
    let mut bytes = [0u8; 65536];
    let mut at = 0;
    while at < size {
        let n = (size - at).min(bytes.len() as u64) as usize;
        file.read_exact_at(&mut bytes[..n], at)?;
        h.update(&bytes[..n]);
        at += n as u64;
    }
    h.update(tail);
    Ok(hex::encode(h.finalize()))
}
fn file_check(file: &File, expected: &Identity, path: &Path) -> io::Result<()> {
    let held = Identity::of(file)?;
    let named = Identity::metadata(&std::fs::symlink_metadata(path)?);
    if &held != expected || named != held {
        return Err(invalid("log inode/link/mode/owner replaced"));
    }
    held.regular(expected.uid)
}
fn copy(src: &File, dst: &File, size: u64) -> io::Result<()> {
    let mut bytes = [0u8; 65536];
    let mut at = 0;
    while at < size {
        let n = (size - at).min(bytes.len() as u64) as usize;
        src.read_exact_at(&mut bytes[..n], at)?;
        dst.write_all_at(&bytes[..n], at)?;
        at += n as u64;
    }
    dst.set_len(size)
}
impl Lease {
    pub(crate) fn open_log(&self, spec: LogSpec<'_>) -> io::Result<LogWriter> {
        if spec.cap_bytes == 0 || spec.cap_bytes > MAX_CAP {
            return Err(invalid("log cap must be1..64MiB"));
        }
        let current = absolute(spec.current)?;
        for path in std::iter::once(spec.current).chain(spec.previous) {
            if path.components().any(|c| {
                c.as_os_str().to_str().is_some_and(|n| {
                    n.starts_with(".iii-")
                        || n.starts_with("backup")
                        || n.starts_with("generation-")
                        || n.ends_with("-compat")
                })
            }) {
                return Err(invalid("log cannot reclassify protected evidence"));
            }
        }
        let previous = spec.previous.map(absolute).transpose()?;
        if previous
            .as_ref()
            .is_some_and(|p| p == &current || p.parent() != current.parent())
        {
            return Err(invalid("log pair must be distinct siblings"));
        }
        // Caller creates exact files ONCE through a normal owned Create/ControlFiles
        // transaction before acquiring a long-lived writer; never reinitializes them.
        self.revalidate()?;
        let parent = Directory::open(current.parent().ok_or_else(|| invalid("log parent"))?)?;
        parent.private(uid())?;
        let open = |p: &Path| {
            open_at(
                &parent.file,
                p.file_name().ok_or_else(|| invalid("log name"))?,
                libc::O_RDWR | libc::O_NONBLOCK,
                0,
            )
        };
        let file = open(&current)?;
        let old = previous.as_ref().map(|p| open(p)).transpose()?;
        let current_id = Identity::of(&file)?;
        current_id.regular(uid())?;
        let previous_id = old.as_ref().map(Identity::of).transpose()?;
        if let Some(id) = &previous_id {
            id.regular(uid())?;
            if id.dev == current_id.dev && id.ino == current_id.ino {
                return Err(invalid("log files alias"));
            }
        }
        let mut tx = self.0.namespace.transaction()?;
        self.validate_continuation(&tx)?;
        let op = tx.operation_mut(&self.0.id)?;
        if self.0.holder != self.0.id || op.closed || op.uncertain || op.cache.is_some() {
            return Err(invalid("log writer requires original available lifecycle"));
        }
        for path in std::iter::once(&current).chain(previous.iter()) {
            if !op
                .entries
                .iter()
                .any(|e| e.path.get().is_ok_and(|p| p == *path) && e.role == PathRole::MutableFile)
            {
                return Err(invalid("log path must be an exact MutableFile declaration"));
            }
            if op.entries.iter().any(|e| {
                e.role == PathRole::Strict
                    && e.path
                        .get()
                        .is_ok_and(|p| super::catalog::overlaps(&p, path))
            }) || op.holds.iter().any(|h| {
                h.path
                    .get()
                    .is_ok_and(|p| super::catalog::overlaps(&p, path))
            }) {
                return Err(busy());
            }
        }
        let current_path = HostPath::new(&current)?;
        let previous_path = previous.as_ref().map(|p| HostPath::new(p)).transpose()?;
        for (path, identity) in std::iter::once((&current, &current_id))
            .chain(previous.as_ref().zip(previous_id.as_ref()))
        {
            let entry = op
                .entries
                .iter()
                .find(|e| e.path.get().is_ok_and(|p| p == *path))
                .ok_or_else(|| invalid("log declaration missing"))?;
            super::validate_entry(entry)?;
            if !entry
                .pins
                .iter()
                .any(|(p, id)| p.get().is_ok_and(|p| p == *path) && id == identity)
            {
                return Err(invalid("log opened a different inode than admitted"));
            }
        }
        let id = if let Some(record) = op.logs.iter_mut().find(|r| r.current == current_path) {
            if record.phase != LogPhase::Finished
                || record.previous != previous_path
                || record.current_id != current_id
                || record.previous_id != previous_id
                || record.cap != spec.cap_bytes
                || record.pending.is_some()
            {
                return Err(invalid(
                    "log history is unresolved or configuration changed",
                ));
            }
            if file.metadata()?.len() != record.cursor.len
                || old
                    .as_ref()
                    .map(|f| f.metadata().map(|m| m.len()))
                    .transpose()?
                    .unwrap_or(0)
                    != record.cursor.previous_len
            {
                return Err(invalid("log size changed after writer finish"));
            }
            if record.current_hash != hash(&file, record.cursor.len, &[])?
                || record.previous_hash
                    != old
                        .as_ref()
                        .map(|f| hash(f, record.cursor.previous_len, &[]))
                        .transpose()?
                || record.current_stamp != log_stamp(&file)?
                || record.previous_stamp != old.as_ref().map(log_stamp).transpose()?
            {
                return Err(invalid("finished log content changed"));
            }
            record.phase = LogPhase::Clean;
            record.holder = self.0.holder.clone();
            record.id.clone()
        } else {
            if op.logs.len() >= MAX_LOGS {
                return Err(limit());
            }
            let len = file.metadata()?.len();
            let previous_len = old
                .as_ref()
                .map(|f| f.metadata().map(|m| m.len()))
                .transpose()?
                .unwrap_or(0);
            if len > MAX_CAP + MAX_RECORD as u64 || previous_len > MAX_CAP + MAX_RECORD as u64 {
                return Err(limit());
            }
            let id = new_id();
            op.logs.push(LogRecord {
                id: id.clone(),
                holder: self.0.holder.clone(),
                current: current_path.clone(),
                previous: previous_path.clone(),
                current_id,
                previous_id,
                cap: spec.cap_bytes,
                phase: LogPhase::Clean,
                cursor: LogCursor {
                    base: previous_len,
                    len,
                    previous_base: 0,
                    previous_len,
                },
                sequence: 0,
                chain: "0".repeat(64),
                pending: None,
                current_hash: hash(&file, len, &[])?,
                previous_hash: old
                    .as_ref()
                    .map(|f| hash(f, previous_len, &[]))
                    .transpose()?,
                current_stamp: log_stamp(&file)?,
                previous_stamp: old.as_ref().map(log_stamp).transpose()?,
            });
            id
        };
        if op.holds.len() + 1 + usize::from(previous.is_some()) > 256 {
            return Err(limit());
        }
        op.epoch = op.epoch.checked_add(1).ok_or_else(limit)?;
        let epoch = op.epoch;
        for path in std::iter::once(current_path).chain(previous_path) {
            op.holds.push(Hold {
                id: id.clone(),
                path,
                kind: HoldKind::Log,
                epoch,
                owner: Some(ResourceOwner {
                    holder: self.0.holder.clone(),
                    runtime: None,
                }),
            });
        }
        tx.save()?;
        drop(tx);
        let hasher = hash_state(&file, file.metadata()?.len())?;
        let result = LogWriter(Arc::new(LogHeld {
            lease: self.clone(),
            id,
            parent,
            current: file,
            previous: old,
            state: Mutex::new(Local {
                hasher,
                failed: false,
                finished: false,
                #[cfg(test)]
                fault: None,
            }),
        }));
        let record = result.0.validate()?;
        if hex::encode(
            result
                .0
                .state
                .lock()
                .map_err(|_| invalid("log mutex poisoned"))?
                .hasher
                .clone()
                .finalize(),
        ) != record.current_hash
        {
            return Err(invalid("log changed during writer issue"));
        }
        Ok(result)
    }
}
impl LogHeld {
    fn validate(&self) -> io::Result<LogRecord> {
        self.parent.revalidate()?;
        let tx = self.lease.0.namespace.transaction()?;
        let op = tx.operation(&self.lease.0.id)?;
        if op.closed || op.uncertain {
            return Err(invalid("log lifecycle unavailable"));
        }
        let record = op
            .logs
            .iter()
            .find(|r| r.id == self.id)
            .ok_or_else(|| invalid("log record missing"))?
            .clone();
        if !op.holds.iter().any(|h| {
            h.id == self.id
                && h.kind == HoldKind::Log
                && h.owner
                    .as_ref()
                    .is_some_and(|o| o.holder == self.lease.0.holder && o.runtime.is_none())
        }) {
            return Err(invalid("log writer no longer owns its hold"));
        }
        file_check(&self.current, &record.current_id, &record.current.get()?)?;
        if let (Some(file), Some(id), Some(path)) =
            (&self.previous, &record.previous_id, &record.previous)
        {
            file_check(file, id, &path.get()?)?;
        }
        if matches!(record.phase, LogPhase::Clean | LogPhase::Finished)
            && (record.current_stamp != log_stamp(&self.current)?
                || record.previous_stamp != self.previous.as_ref().map(log_stamp).transpose()?)
        {
            return Err(invalid("log bytes changed outside owned effect"));
        }
        Ok(record)
    }
    fn fail(&self, local: &mut Local) {
        local.failed = true;
        if let Ok(mut tx) = self.lease.0.namespace.transaction() {
            if let Ok(op) = tx.operation_mut(&self.lease.0.id) {
                op.uncertain = true;
                if let Some(log) = op.logs.iter_mut().find(|r| r.id == self.id) {
                    log.phase = LogPhase::Failed;
                }
                let _ = tx.save();
            }
        }
    }
}
impl LogWriter {
    pub(crate) fn append(&self, record: &[u8]) -> io::Result<LogCursor> {
        let mut local = self
            .0
            .state
            .lock()
            .map_err(|_| invalid("log mutex poisoned"))?;
        if local.failed || local.finished {
            return Err(invalid("log writer is terminal"));
        }
        let result = (|| -> io::Result<LogCursor> {
            if record.len() > MAX_RECORD {
                return Err(limit());
            }
            let before = self.0.validate()?;
            if before.phase != LogPhase::Clean || before.pending.is_some() {
                return Err(invalid("unresolved log outcome"));
            }
            if self.0.current.metadata()?.len() != before.cursor.len
                || self
                    .0
                    .previous
                    .as_ref()
                    .map(|f| f.metadata().map(|m| m.len()))
                    .transpose()?
                    .unwrap_or(0)
                    != before.cursor.previous_len
            {
                return Err(invalid("log cursor/file size mismatch"));
            }
            let size = before
                .cursor
                .len
                .checked_add(record.len() as u64)
                .ok_or_else(limit)?;
            let rotate = size > before.cap;
            if rotate && self.0.previous.is_none() {
                return Err(invalid(
                    "single-file log cap reached; bounded retention rotation path required",
                ));
            }
            let end = before.cursor.base.checked_add(size).ok_or_else(limit)?;
            let next = if rotate {
                LogCursor {
                    base: end,
                    len: 0,
                    previous_base: before.cursor.base,
                    previous_len: size,
                }
            } else {
                LogCursor {
                    len: size,
                    ..before.cursor.clone()
                }
            };
            let mut accumulated = local.hasher.clone();
            accumulated.update(record);
            let data_hash = hex::encode(accumulated.clone().finalize());
            let current_hash = if rotate {
                hex::encode(Sha256::digest([]))
            } else {
                data_hash.clone()
            };
            let previous_hash = if rotate {
                Some(data_hash)
            } else {
                before.previous_hash.clone()
            };
            let mut chain_hash = Sha256::new();
            chain_hash.update(before.chain.as_bytes());
            chain_hash.update((record.len() as u64).to_le_bytes());
            chain_hash.update(Sha256::digest(record));
            let chain = hex::encode(chain_hash.finalize());
            {
                let mut tx = self.0.lease.0.namespace.transaction()?;
                let op = tx.operation_mut(&self.0.lease.0.id)?;
                let row = op
                    .logs
                    .iter_mut()
                    .find(|r| r.id == self.0.id)
                    .ok_or_else(|| invalid("log missing"))?;
                if row.phase != LogPhase::Clean || row.cursor != before.cursor {
                    return Err(invalid("log generation changed"));
                }
                row.phase = LogPhase::Pending;
                row.pending = Some(LogPending {
                    reset: false,
                    next: next.clone(),
                    current_hash,
                    previous_hash,
                    chain,
                });
                tx.save()?;
            }
            #[cfg(test)]
            if local.fault == Some("intent") {
                return Err(io::Error::other("injected log intent"));
            }
            self.0.current.write_all_at(record, before.cursor.len)?;
            self.0.current.sync_all()?;
            #[cfg(test)]
            if local.fault == Some("append") {
                return Err(io::Error::other("injected log append"));
            }
            if rotate {
                let old = self.0.previous.as_ref().unwrap();
                copy(&self.0.current, old, size)?;
                old.sync_all()?;
                #[cfg(test)]
                if local.fault == Some("rotation") {
                    return Err(io::Error::other("injected log rotation"));
                }
                self.0.current.set_len(0)?;
                self.0.current.sync_all()?;
            }
            #[cfg(test)]
            if local.fault == Some("outcome") {
                return Err(io::Error::other("injected log outcome"));
            }
            let verified = self.0.validate()?;
            let pending = verified
                .pending
                .as_ref()
                .ok_or_else(|| invalid("log intent lost"))?;
            if self.0.current.metadata()?.len() != pending.next.len
                || self
                    .0
                    .previous
                    .as_ref()
                    .map(|f| f.metadata().map(|m| m.len()))
                    .transpose()?
                    .unwrap_or(0)
                    != pending.next.previous_len
            {
                return Err(invalid("log effect size mismatch"));
            }
            let mut tx = self.0.lease.0.namespace.transaction()?;
            let row = tx
                .operation_mut(&self.0.lease.0.id)?
                .logs
                .iter_mut()
                .find(|r| r.id == self.0.id)
                .ok_or_else(|| invalid("log missing"))?;
            if row.phase != LogPhase::Pending || row.sequence != before.sequence {
                return Err(invalid("log outcome raced"));
            }
            row.cursor = next.clone();
            row.chain = pending.chain.clone();
            row.sequence = row.sequence.checked_add(1).ok_or_else(limit)?;
            row.current_hash = pending.current_hash.clone();
            row.previous_hash = pending.previous_hash.clone();
            row.current_stamp = log_stamp(&self.0.current)?;
            row.previous_stamp = self.0.previous.as_ref().map(log_stamp).transpose()?;
            row.pending = None;
            row.phase = LogPhase::Clean;
            tx.save()?;
            local.hasher = if rotate { Sha256::new() } else { accumulated };
            Ok(next)
        })();
        if result.is_err() {
            self.0.fail(&mut local);
        }
        result
    }
    /// Explicit ordinary-log retention policy for a new caller instance. It is
    /// not migration cleanup: only this exact log pair, no hidden prior files.
    pub(crate) fn reset_instance(&self) -> io::Result<LogCursor> {
        let mut local = self
            .0
            .state
            .lock()
            .map_err(|_| invalid("log mutex poisoned"))?;
        if local.failed || local.finished || Arc::strong_count(&self.0) != 1 {
            return Err(invalid("log reset requires sole active owner"));
        }
        let result = (|| -> io::Result<LogCursor> {
            let before = self.0.validate()?;
            if before.phase != LogPhase::Clean {
                return Err(invalid("pending log reset"));
            }
            let next = LogCursor {
                base: 0,
                len: 0,
                previous_base: 0,
                previous_len: 0,
            };
            let empty = hex::encode(Sha256::digest([]));
            let chain = hex::encode(Sha256::digest(
                format!("{}:explicit-reset:{}", before.chain, before.sequence).as_bytes(),
            ));
            {
                let mut tx = self.0.lease.0.namespace.transaction()?;
                let row = tx
                    .operation_mut(&self.0.lease.0.id)?
                    .logs
                    .iter_mut()
                    .find(|r| r.id == self.0.id)
                    .ok_or_else(|| invalid("log missing"))?;
                row.phase = LogPhase::Pending;
                row.pending = Some(LogPending {
                    reset: true,
                    next: next.clone(),
                    current_hash: empty.clone(),
                    previous_hash: self.0.previous.as_ref().map(|_| empty.clone()),
                    chain: chain.clone(),
                });
                tx.save()?;
            }
            self.0.current.set_len(0)?;
            self.0.current.sync_all()?;
            if let Some(old) = &self.0.previous {
                old.set_len(0)?;
                old.sync_all()?;
            }
            self.0.validate()?;
            let mut tx = self.0.lease.0.namespace.transaction()?;
            let row = tx
                .operation_mut(&self.0.lease.0.id)?
                .logs
                .iter_mut()
                .find(|r| r.id == self.0.id)
                .ok_or_else(|| invalid("log missing"))?;
            row.cursor = next.clone();
            row.current_hash = empty.clone();
            row.previous_hash = self.0.previous.as_ref().map(|_| empty);
            row.current_stamp = log_stamp(&self.0.current)?;
            row.previous_stamp = self.0.previous.as_ref().map(log_stamp).transpose()?;
            row.chain = chain;
            row.sequence = row.sequence.checked_add(1).ok_or_else(limit)?;
            row.pending = None;
            row.phase = LogPhase::Clean;
            tx.save()?;
            local.hasher = Sha256::new();
            Ok(next)
        })();
        if result.is_err() {
            self.0.fail(&mut local);
        }
        result
    }
    fn cursor_inner(&self) -> io::Result<LogCursor> {
        let local = self
            .0
            .state
            .lock()
            .map_err(|_| invalid("log mutex poisoned"))?;
        if local.failed {
            return Err(invalid("log failed"));
        }
        Ok(self.0.validate()?.cursor)
    }
    fn read_inner(&self, offset: u64, max_bytes: usize) -> io::Result<LogRead> {
        let local = self
            .0
            .state
            .lock()
            .map_err(|_| invalid("log mutex poisoned"))?;
        if local.failed || max_bytes > MAX_RECORD {
            return Err(invalid("invalid log read"));
        }
        let row = self.0.validate()?;
        if row.phase != LogPhase::Clean {
            return Err(invalid("log pending"));
        }
        let oldest = if self.0.previous.is_some() {
            row.cursor.previous_base
        } else {
            row.cursor.base
        };
        let mut at = offset.max(oldest).min(row.cursor.end());
        let mut result = Vec::new();
        for (file, base, len) in self
            .0
            .previous
            .iter()
            .map(|f| (f, row.cursor.previous_base, row.cursor.previous_len))
            .chain(std::iter::once((
                &self.0.current,
                row.cursor.base,
                row.cursor.len,
            )))
        {
            if at < base + len && at.max(base) - base <= len && result.len() < max_bytes {
                let start = at.max(base) - base;
                let n = (len - start).min((max_bytes - result.len()) as u64) as usize;
                let old = result.len();
                result.resize(old + n, 0);
                file.read_exact_at(&mut result[old..], start)?;
                at = base + start + n as u64;
            }
        }
        Ok(LogRead {
            bytes: result,
            next: at,
            end: row.cursor.end(),
            oldest,
        })
    }
    fn terminal_read<T>(&self, result: io::Result<T>) -> io::Result<T> {
        if result.is_err() {
            let mut local = self
                .0
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.0.fail(&mut local);
        }
        result
    }
    pub(crate) fn cursor(&self) -> io::Result<LogCursor> {
        self.terminal_read(self.cursor_inner())
    }
    pub(crate) fn read(&self, offset: u64, max_bytes: usize) -> io::Result<LogRead> {
        self.terminal_read(self.read_inner(offset, max_bytes))
    }
    pub(crate) fn finish(self) -> io::Result<LogCursor> {
        if Arc::strong_count(&self.0) != 1 {
            return Err(invalid("log writer clones still active"));
        }
        let mut local = self
            .0
            .state
            .lock()
            .map_err(|_| invalid("log mutex poisoned"))?;
        if local.failed || local.finished {
            return Err(invalid("log cannot finish"));
        }
        let row = self.0.validate()?;
        if row.phase != LogPhase::Clean || row.pending.is_some() {
            return Err(invalid("log outcome pending"));
        }
        if hash(&self.0.current, row.cursor.len, &[])? != row.current_hash
            || self
                .0
                .previous
                .as_ref()
                .map(|f| hash(f, row.cursor.previous_len, &[]))
                .transpose()?
                != row.previous_hash
        {
            return Err(invalid("log finish bytes do not match completed outcomes"));
        }
        self.0.current.sync_all()?;
        if let Some(old) = &self.0.previous {
            old.sync_all()?;
        }
        let mut tx = self.0.lease.0.namespace.transaction()?;
        let op = tx.operation_mut(&self.0.lease.0.id)?;
        op.logs
            .iter_mut()
            .find(|r| r.id == self.0.id)
            .ok_or_else(|| invalid("log missing"))?
            .phase = LogPhase::Finished;
        op.holds
            .retain(|h| !(h.id == self.0.id && h.kind == HoldKind::Log));
        tx.save()?;
        local.finished = true;
        Ok(row.cursor)
    }
    #[cfg(test)]
    pub(super) fn fault(&self, at: &'static str) {
        self.0.state.lock().unwrap().fault = Some(at);
    }
}
impl Drop for LogHeld {
    fn drop(&mut self) {
        let unfinished = self.state.get_mut().map_or(true, |s| !s.finished);
        if unfinished {
            let _ = self.lease.finish_hold(&self.id, true);
        }
    }
}
pub(super) fn validate_record(op: &super::catalog::Operation) -> io::Result<()> {
    if op.logs.len() > MAX_LOGS {
        return Err(limit());
    }
    let mut paths = std::collections::BTreeSet::new();
    let mut ids = std::collections::BTreeSet::new();
    for row in &op.logs {
        if !ids.insert(&row.id)
            || !id_valid(&row.id)
            || !id_valid(&row.holder)
            || row.cap == 0
            || row.cap > MAX_CAP
            || row.chain.len() != 64
            || !paths.insert(row.current.get()?)
            || row.cursor.len > MAX_CAP + MAX_RECORD as u64
            || row.cursor.previous_len > MAX_CAP + MAX_RECORD as u64
        {
            return Err(invalid("invalid log record"));
        }
        if row.cursor.base.checked_add(row.cursor.len).is_none()
            || row
                .cursor
                .previous_base
                .checked_add(row.cursor.previous_len)
                != Some(row.cursor.base)
            || row.current_hash.len() != 64
            || row.previous_hash.as_ref().is_some_and(|h| h.len() != 64)
        {
            return Err(invalid("invalid log cursor/hash"));
        }
        let holds = op
            .holds
            .iter()
            .filter(|h| h.kind == HoldKind::Log && h.id == row.id)
            .collect::<Vec<_>>();
        if row.phase == LogPhase::Finished {
            if !holds.is_empty() {
                return Err(invalid("finished log retains writer hold"));
            }
        } else if holds.len() != 1 + usize::from(row.previous.is_some())
            || holds.iter().any(|h| {
                h.owner
                    .as_ref()
                    .is_none_or(|o| o.holder != row.holder || o.runtime.is_some())
                    || !(h.path == row.current || row.previous.as_ref() == Some(&h.path))
            })
        {
            return Err(invalid("log issuer/hold mismatch"));
        }
        if row.phase != LogPhase::Finished && !op.holder_ids.contains(&row.holder) {
            return Err(invalid("log holder no longer in operation"));
        }
        if let Some(pending) = &row.pending {
            if pending.current_hash.len() != 64
                || pending.chain.len() != 64
                || pending
                    .previous_hash
                    .as_ref()
                    .is_some_and(|s| s.len() != 64)
                || (!pending.reset
                    && (pending.next.end() < row.cursor.end()
                        || pending.next.end() - row.cursor.end() > MAX_RECORD as u64))
                || (pending.reset && pending.next.end() != 0)
                || pending.next.len > MAX_CAP + MAX_RECORD as u64
                || pending.next.previous_len > MAX_CAP + MAX_RECORD as u64
                || pending
                    .next
                    .previous_base
                    .checked_add(pending.next.previous_len)
                    != Some(pending.next.base)
            {
                return Err(invalid("invalid bounded log intent"));
            }
        }
        row.current_id.regular(op.sentinel.uid)?;
        if row.previous.is_some() != row.previous_id.is_some()
            || row.previous.is_some() != row.previous_hash.is_some()
            || row.previous.is_some() != row.previous_stamp.is_some()
        {
            return Err(invalid("partial log pair record"));
        }
        if let Some(path) = &row.previous {
            if path.get()?.parent() != row.current.get()?.parent() {
                return Err(invalid("log pair parent changed"));
            }
            if !paths.insert(path.get()?) {
                return Err(invalid("overlapping log record"));
            }
            row.previous_id
                .as_ref()
                .ok_or_else(|| invalid("missing log identity"))?
                .regular(op.sentinel.uid)?;
        }
        if row.phase == LogPhase::Pending && row.pending.is_none() {
            return Err(invalid("log pending without intent"));
        }
        if matches!(row.phase, LogPhase::Clean | LogPhase::Finished) && row.pending.is_some() {
            return Err(invalid("completed log has pending effect"));
        }
    }
    Ok(())
}
pub(super) fn reconcile(op: &mut super::catalog::Operation) -> io::Result<bool> {
    for row in &mut op.logs {
        if row.phase == LogPhase::Finished {
            continue;
        }
        let path = row.current.get()?;
        let parent = Directory::open(
            path.parent()
                .ok_or_else(|| invalid("log recovery parent"))?,
        )?;
        parent.private(op.sentinel.uid)?;
        let current = open_at(
            &parent.file,
            path.file_name().ok_or_else(|| invalid("log name"))?,
            libc::O_RDWR | libc::O_NONBLOCK,
            0,
        )?;
        file_check(&current, &row.current_id, &path)?;
        let old = row
            .previous
            .as_ref()
            .map(|p| -> io::Result<File> {
                let p = p.get()?;
                let file = open_at(
                    &parent.file,
                    p.file_name().ok_or_else(|| invalid("old log name"))?,
                    libc::O_RDWR | libc::O_NONBLOCK,
                    0,
                )?;
                file_check(
                    &file,
                    row.previous_id
                        .as_ref()
                        .ok_or_else(|| invalid("old log identity"))?,
                    &p,
                )?;
                Ok(file)
            })
            .transpose()?;
        if row.phase == LogPhase::Clean && row.pending.is_none() {
            if current.metadata()?.len() != row.cursor.len
                || hash(&current, row.cursor.len, &[])? != row.current_hash
                || old
                    .as_ref()
                    .map(|f| hash(f, row.cursor.previous_len, &[]))
                    .transpose()?
                    != row.previous_hash
            {
                return Ok(false);
            }
            current.sync_all()?;
            if let Some(old) = &old {
                old.sync_all()?;
            }
            row.phase = LogPhase::Finished;
            op.holds
                .retain(|h| !(h.id == row.id && h.kind == HoldKind::Log));
            continue;
        }
        let Some(pending) = &row.pending else {
            return Ok(false);
        };
        let rotated = pending.reset || pending.next.base != row.cursor.base;
        let proof = hash(&current, pending.next.len, &[])?;
        if current.metadata()?.len() != pending.next.len || proof != pending.current_hash {
            return Ok(false);
        }
        if let Some(old) = &old {
            if old.metadata()?.len() != pending.next.previous_len
                || (rotated
                    && Some(hash(old, pending.next.previous_len, &[])?) != pending.previous_hash)
            {
                return Ok(false);
            }
            old.sync_all()?;
        }
        current.sync_all()?;
        row.cursor = pending.next.clone();
        row.chain = pending.chain.clone();
        row.sequence = row.sequence.checked_add(1).ok_or_else(limit)?;
        row.current_hash = pending.current_hash.clone();
        row.previous_hash = pending.previous_hash.clone();
        row.current_stamp = log_stamp(&current)?;
        row.previous_stamp = old.as_ref().map(log_stamp).transpose()?;
        row.pending = None;
        row.phase = LogPhase::Finished;
        op.holds
            .retain(|h| !(h.id == row.id && h.kind == HoldKind::Log));
    }
    Ok(true)
}
#[cfg(test)]
#[path = "log_tests.rs"]
mod tests;
