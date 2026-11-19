// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Rootfs cache.
//!
//! One content-addressable cache at `~/.iii/cache/<slug>/`, keyed on the
//! OCI reference itself. Worker VMs and sandboxes both go through
//! `ensure_rootfs`, so an image pulled for one satisfies the other without
//! re-pulling.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::cli::oci_ref::{canonical_cache_key, pinned_digest};
use crate::cli::worker_manager::oci::{pull_and_extract_rootfs_checked, rootfs_slug_for_image};

/// Required validation at writer boundaries; never a mutation/refresh grant.
/// These checkpoints do not make pathname operations atomic against outsiders.
/// The closure owns its fence so a queued blocking lock task retains it too.
#[derive(Clone)]
pub(crate) struct Checkpoint {
    state: std::sync::Arc<CheckpointState>,
    #[cfg(test)]
    on_contention: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
}

struct CheckpointState {
    validate: Box<dyn Fn() -> std::io::Result<()> + Send + Sync>,
    // Serialize checks and diagnostic effects. Once set, never call validate
    // again, even if the underlying path or transient I/O error recovers.
    failure: std::sync::Mutex<Option<AdmissionRefused>>,
}

/// Typed across anyhow contexts/stream helpers; retains the original kind,
/// OS code and source rather than classifying a diagnostic string.
#[derive(Clone, Debug)]
pub(crate) struct AdmissionRefused(std::sync::Arc<std::io::Error>);

impl std::fmt::Display for AdmissionRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "worker admission refused: {}", self.0)
    }
}
impl std::error::Error for AdmissionRefused {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

pub(crate) fn is_admission_refusal(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| cause.is::<AdmissionRefused>())
}

impl Checkpoint {
    pub(crate) fn new(validate: impl Fn() -> std::io::Result<()> + Send + Sync + 'static) -> Self {
        Self {
            state: std::sync::Arc::new(CheckpointState {
                validate: Box::new(validate),
                failure: std::sync::Mutex::new(None),
            }),
            #[cfg(test)]
            on_contention: None,
        }
    }

    pub(crate) fn check(&self) -> Result<(), AdmissionRefused> {
        self.with_valid(|| ())
    }

    pub(crate) fn with_valid<T>(&self, effect: impl FnOnce() -> T) -> Result<T, AdmissionRefused> {
        self.with_io(|| Ok(effect()))
    }

    /// Restricted diagnostic operations can fail after validation. Latch that
    /// failure too, without a second evaluation or a cleanup recovery attempt.
    pub(crate) fn with_io<T>(
        &self,
        effect: impl FnOnce() -> std::io::Result<T>,
    ) -> Result<T, AdmissionRefused> {
        let mut failure = match self.state.failure.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                guard.get_or_insert_with(|| {
                    AdmissionRefused(std::sync::Arc::new(std::io::Error::other(
                        "checkpoint validation panicked",
                    )))
                });
                guard
            }
        };
        if let Some(error) = failure.as_ref() {
            return Err(error.clone());
        }
        if let Err(cause) = (self.state.validate)() {
            let error = AdmissionRefused(std::sync::Arc::new(cause));
            *failure = Some(error.clone());
            return Err(error);
        }
        // Keep the mutex through diagnostic mutation: a concurrent refusal
        // cannot be recorded before a supposedly-authorized cleanup/write.
        match effect() {
            Ok(value) => Ok(value),
            Err(cause) => {
                let error = AdmissionRefused(std::sync::Arc::new(cause));
                *failure = Some(error.clone());
                Err(error)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn on_contention(mut self, notify: impl Fn() + Send + Sync + 'static) -> Self {
        self.on_contention = Some(std::sync::Arc::new(notify));
        self
    }
}

/// Existing cache lock protocol (same file and flock), split so validation can
/// run AFTER acquisition but BEFORE lock_with_pid's otherwise immediate write.
/// fslock also truncates on Drop, so rejected acquisitions must not use its Drop.
pub(crate) struct CheckedCacheLock {
    file: std::fs::File,
    clear_on_drop: bool,
    checkpoint: Checkpoint,
}
impl CheckedCacheLock {
    /// Successful users must finish explicitly: Drop cannot report a new
    /// admission refusal or cleanup error to their caller. Suppress its fallback
    /// before trying cleanup so an error cannot trigger a second cleanup attempt.
    pub(crate) fn finish(mut self) -> Result<()> {
        self.clear_on_drop = false;
        self.checkpoint
            .with_valid(|| self.file.set_len(0))?
            .context("could not clear the held cache-lock diagnostic")?;
        // self drops here, releasing flock through its already-held descriptor.
        Ok(())
    }
}

impl Drop for CheckedCacheLock {
    fn drop(&mut self) {
        // Invalidated admission must not truncate even this diagnostic file.
        // A successful/ordinary error path keeps the existing empty-on-drop rule.
        if self.clear_on_drop {
            let _ = self.checkpoint.with_valid(|| self.file.set_len(0));
        }
        // Closing the owned descriptor releases flock even on early refusal.
    }
}

pub(crate) fn checked_cache_lock(
    path: &Path,
    checkpoint: &Checkpoint,
    write_pid: bool,
) -> Result<CheckedCacheLock> {
    use std::io::{Seek, SeekFrom, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    checkpoint.check()?; // Before open/create, including after spawn_blocking's queue.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o644) // Match fslock's existing creation mode (subject to umask).
        .truncate(false)
        .open(path)
        .with_context(|| format!("could not open {}", path.display()))?;
    let mut held = CheckedCacheLock {
        file,
        clear_on_drop: false,
        checkpoint: checkpoint.clone(),
    };
    checkpoint.check()?; // Last pre-wait boundary; opening alone cannot authorize writes.
    // SAFETY: the descriptor stays owned through both attempts and release.
    if unsafe { libc::flock(held.file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::WouldBlock {
            return Err(error).with_context(|| format!("could not lock {}", path.display()));
        }
        // Notify only AFTER an actual contended attempt, never before flock.
        #[cfg(test)]
        if let Some(notify) = &checkpoint.on_contention {
            notify();
        }
        if unsafe { libc::flock(held.file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("could not lock {}", path.display()));
        }
    }
    checkpoint
        .with_valid(|| -> std::io::Result<()> {
            // Under the acquired lock and terminal-state mutex, BEFORE any write.
            held.clear_on_drop = true;
            if write_pid {
                held.file.set_len(0)?;
                held.file.seek(SeekFrom::Start(0))?;
                writeln!(held.file, "{}", std::process::id())?;
            }
            Ok(())
        })?
        .with_context(|| format!("could not lock {}", path.display()))?;
    Ok(held)
}

/// Diagnostic adapter used by worker cache/EROFS mutations. The legacy File
/// route is independent only; a refused mutation never falls back to it.
pub(crate) struct MutationLock {
    lock: Option<crate::worker_admission::DiagnosticLock>,
    checkpoint: Checkpoint,
}
impl MutationLock {
    pub(crate) fn acquire(
        mut lock: crate::worker_admission::DiagnosticLock,
        checkpoint: Checkpoint,
        write_pid: bool,
    ) -> Result<Self> {
        checkpoint.check()?;
        if !checkpoint.with_io(|| lock.try_lock_exclusive())? {
            #[cfg(test)]
            if let Some(notify) = &checkpoint.on_contention {
                notify();
            }
            // Neither the terminal mutex nor a mutation/catalog mutex is held.
            let acquired = lock.lock_exclusive();
            checkpoint.with_io(|| acquired)?;
        }
        checkpoint.with_io(|| -> std::io::Result<()> {
            lock.revalidate()?;
            if write_pid {
                lock.write_pid(std::process::id())?;
            }
            Ok(())
        })?;
        Ok(Self {
            lock: Some(lock),
            checkpoint,
        })
    }

    pub(crate) fn finish(mut self) -> Result<()> {
        if let Some(mut lock) = self.lock.take() {
            self.checkpoint.with_io(|| -> std::io::Result<()> {
                lock.revalidate()?;
                lock.clear()
            })?;
            drop(lock); // Must precede MutationGuard::commit.
        }
        Ok(())
    }
    // Drop intentionally performs no diagnostic write or pathname cleanup.
}

/// One caller-owned core mutation. The callback checks THIS guard's original
/// proofs during staging, not stale pre-publication inode pins on a fresh lease.
pub(crate) struct MutationWriter {
    guard: std::sync::Arc<std::sync::Mutex<crate::worker_admission::MutationGuard>>,
    pub(crate) checkpoint: Checkpoint,
}
impl MutationWriter {
    pub(crate) fn begin(
        lease: &crate::worker_admission::Lease,
        plan: crate::worker_admission::MutationPlan,
    ) -> std::io::Result<Self> {
        Ok(Self::from_guard(lease.begin(plan)?))
    }

    pub(crate) fn from_guard(guard: crate::worker_admission::MutationGuard) -> Self {
        let guard = std::sync::Arc::new(std::sync::Mutex::new(guard));
        let checked = guard.clone();
        let checkpoint = Checkpoint::new(move || {
            checked
                .lock()
                .map_err(|_| std::io::Error::other("mutation poisoned"))?
                .checkpoint()
        });
        Self { guard, checkpoint }
    }

    pub(crate) fn apply<T>(
        &self,
        effect: impl FnOnce(&mut crate::worker_admission::MutationGuard) -> std::io::Result<T>,
    ) -> Result<T> {
        self.checkpoint
            .with_io(|| {
                let mut guard = self
                    .guard
                    .lock()
                    .map_err(|_| std::io::Error::other("mutation poisoned"))?;
                effect(&mut guard)
            })
            .map_err(Into::into)
    }

    pub(crate) fn diagnostic(
        &self,
        path: &Path,
    ) -> Result<crate::worker_admission::DiagnosticLock> {
        self.apply(|guard| guard.diagnostic_lock(path))
    }

    pub(crate) fn commit(self) -> Result<()> {
        self.checkpoint.check()?;
        drop(self.checkpoint);
        let guard = std::sync::Arc::try_unwrap(self.guard)
            .map_err(|_| anyhow::anyhow!("mutation still has checkpoint/writer references"))?
            .into_inner()
            .map_err(|_| anyhow::anyhow!("mutation poisoned"))?;
        guard.commit()?;
        Ok(())
    }
}

/// Canonical cache root. All new pulls land under here, keyed by
/// `rootfs_slug_for_image(oci_ref)`.
pub(crate) fn cache_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".iii")
        .join("cache")
}

/// Canonical on-disk location for `oci_ref`'s rootfs. Two different
/// flows referencing the same image resolve to the same directory.
/// Digest-pinned refs (`foo:tag@sha256:...`) collapse to the unpinned
/// form's dir via `canonical_cache_key` while that dir's digest sidecar
/// lists the pin, so engine-resolved pins never trigger a re-pull of an
/// image the user already added. While that dir is empty a pin pulls into
/// it, so a later unpinned use reuses that pull. Any other pin gets a dir
/// of its own: a changed pin must miss, not be served another digest's
/// rootfs. A digest-only pin (`foo@sha256:...`) names no tag, so it never
/// shares or fills `foo:latest`'s dir.
// shortcut: per-digest dirs are never evicted, add a cache GC if pin churn grows ~/.iii/cache.
pub fn canonical_path(oci_ref: &str) -> PathBuf {
    canonical_path_at(&cache_root(), oci_ref)
}

pub(crate) fn canonical_path_at(cache: &Path, oci_ref: &str) -> PathBuf {
    let shared = cache.join(rootfs_slug_for_image(canonical_cache_key(oci_ref)));
    match pinned_digest(oci_ref) {
        Some(pin) if is_populated(&shared) && !pulled_from(&shared, pin) => {
            cache.join(rootfs_slug_for_image(oci_ref))
        }
        _ => shared,
    }
}

/// `<rootfs>.digest`, beside the rootfs: the manifest digests it was
/// pulled from, one per line.
pub(crate) fn digest_path(rootfs: &Path) -> PathBuf {
    let mut name = rootfs.file_name().unwrap_or_default().to_os_string();
    name.push(".digest");
    rootfs.with_file_name(name)
}

/// True if the rootfs at `rootfs` was pulled from manifest `digest`.
fn pulled_from(rootfs: &Path, digest: &str) -> bool {
    std::fs::read_to_string(digest_path(rootfs))
        .is_ok_and(|digests| digests.lines().any(|d| d.eq_ignore_ascii_case(digest)))
}

/// The manifest digest the cached rootfs of `oci_ref` was pulled from: the one
/// the reference resolved to, the first its sidecar lists. `None` when the
/// cache has no sidecar for it.
pub fn cached_digest(oci_ref: &str) -> Option<String> {
    let digests = std::fs::read_to_string(digest_path(&canonical_path(oci_ref))).ok()?;
    digests
        .lines()
        .map(str::trim)
        .find(|d| d.starts_with("sha256:"))
        .map(str::to_string)
}

/// True if `path` looks like a populated OCI rootfs. The `bin/` check
/// catches stub directories left behind by a failed pull — every sane
/// Linux image ships `/bin/sh`, so a rootfs with no `bin/` is either
/// mid-extract or arch-mismatched.
pub fn is_populated(path: &Path) -> bool {
    path.exists() && path.join("bin").exists()
}

/// The populated cache path for `oci_ref`, or `None` on a miss.
pub fn resolve_cached(oci_ref: &str) -> Option<PathBuf> {
    let canonical = canonical_path(oci_ref);
    is_populated(&canonical).then_some(canonical)
}

/// Lock file guarding one image's pull. Lives beside the rootfs
/// directories rather than inside one, because the pull begins by
/// deleting that directory. Pinned and unpinned variants share it, so
/// which dir `canonical_path` picks cannot change while it is held.
#[cfg(test)]
fn lock_path_for_image(oci_ref: &str) -> PathBuf {
    lock_path_at(&cache_root(), oci_ref)
}

pub(crate) fn lock_path_at(cache: &Path, oci_ref: &str) -> PathBuf {
    let mut path = cache.join(rootfs_slug_for_image(canonical_cache_key(oci_ref)));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "rootfs".to_string());
    path.set_file_name(format!("{name}.lock"));
    path
}

/// Take the per-image pull lock, waiting for any other holder to finish.
///
/// Acquired on a blocking thread, like the bundle installer's staging
/// lock: `fslock` blocks on a syscall and the async runtime must not.
/// The lock is released when the guard drops, and by the kernel if the
/// process dies, so a crashed pull does not wedge the next one.
async fn lock_at_path(lock_path: PathBuf, checkpoint: Checkpoint) -> Result<CheckedCacheLock> {
    checkpoint.check()?; // Parent creation is an effect too.
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "could not create the rootfs cache directory {}",
                parent.display()
            )
        })?;
    }
    tokio::task::spawn_blocking(move || checked_cache_lock(&lock_path, &checkpoint, true))
        .await
        .context("the rootfs pull lock task panicked")?
}

/// Ensure a populated rootfs exists for `oci_ref`; return its on-disk
/// path. `on_pull_start` fires exactly once, only on a cache miss, right
/// before the network pull begins — use it to log "Pulling..." or
/// surface a progress event to the user.
pub async fn ensure_rootfs(oci_ref: &str, on_pull_start: impl FnOnce()) -> Result<PathBuf> {
    // Compatibility entry for independent, non-worker callers only.
    ensure_rootfs_checked(oci_ref, on_pull_start, Checkpoint::new(|| Ok(()))).await
}

pub(crate) async fn ensure_rootfs_checked(
    oci_ref: &str,
    on_pull_start: impl FnOnce(),
    checkpoint: Checkpoint,
) -> Result<PathBuf> {
    ensure_rootfs_at(oci_ref, on_pull_start, &cache_root(), checkpoint, |image, dest, check| async move {
        pull_and_extract_rootfs_checked(&image, &dest, check).await
    }).await
}

async fn ensure_rootfs_at<F, Fut>(
    oci_ref: &str,
    on_pull_start: impl FnOnce(),
    cache: &Path,
    checkpoint: Checkpoint,
    pull: F,
) -> Result<PathBuf>
where
    F: FnOnce(String, PathBuf, Checkpoint) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<String>>>,
{
    checkpoint.check()?;
    let selected = canonical_path_at(cache, oci_ref);
    if is_populated(&selected) {
        return Ok(selected);
    }
    // Past this point we are going to delete and re-extract `dest`, so no
    // second caller may be inside the same image's miss path. Two of them
    // (a prefetch and the spawn it was meant to warm, or simply two `iii`
    // processes) would each `remove_dir_all` a directory the other was
    // extracting into, and the survivor would keep a rootfs with another
    // pull's files spliced through it.
    let lock = lock_at_path(lock_path_at(cache, oci_ref), checkpoint.clone()).await?;
    checkpoint.check()?; // Also covers rescheduling after the blocking lock returned.
    // Re-check under the lock: whoever held it before us has very likely
    // just finished this exact pull, and re-pulling would throw away a
    // rootfs that is already complete.
    let selected = canonical_path_at(cache, oci_ref);
    if is_populated(&selected) {
        lock.finish()?;
        return Ok(selected);
    }
    on_pull_start();
    checkpoint.check()?;
    let dest = selected;
    // Clean any partial-state stub from a prior failed attempt: a
    // dir-exists-but-empty rootfs can confound tar's overwrite logic
    // and fool `is_populated` on the next call.
    if dest.exists() {
        let _ = std::fs::remove_dir_all(&dest);
    }
    // Drop any stale sibling `<slug>.erofs` so the freshly extracted rootfs is
    // never paired with an overlay lower built from the previous contents.
    // (ensure_base_erofs also rebuilds on stale mtime; this is the
    // belt-and-suspenders explicit GC at the re-extract boundary.)
    crate::cli::erofs::remove_base_erofs(&dest);
    // A stale sidecar must never vouch for whatever this pull leaves.
    let digest_file = digest_path(&dest);
    let _ = std::fs::remove_file(&digest_file);
    let digests = pull(oci_ref.to_string(), dest.clone(), checkpoint.clone()).await?;
    // Without the sidecar a pinned ref only misses this dir and pulls its own.
    record_digest(&digest_file, &digests, &checkpoint)?;
    lock.finish()?;
    Ok(dest)
}

#[derive(Clone)]
pub(crate) struct PreparedCache {
    pub(crate) lease: crate::worker_admission::Lease,
    pub(crate) reference: String,
    pub(crate) root: PathBuf,
    pub(crate) lower: PathBuf,
    env: Vec<(String, String)>,
}
impl PreparedCache {
    pub(crate) fn validate(&self, reference: &str) -> Result<()> {
        anyhow::ensure!(
            self.reference == reference,
            "prepared cache reference changed; H1 continuation is required"
        );
        self.lease.revalidate()?;
        let root = self.lease.use_path(&self.root)?;
        let lower = self.lease.use_path(&self.lower)?;
        root.revalidate()?;
        lower.revalidate()?;
        Ok(())
    }
    pub(crate) fn env(&self) -> Result<Vec<(String, String)>> {
        self.validate(&self.reference)?;
        Ok(self.env.clone())
    }
}

/// Short, separate namespace operation, before the consumer is admitted. A
/// warm private parent is only checked; no chmod, diagnostic or LRU writes.
pub(crate) fn prepare_cache_parent(
    context: &crate::worker_admission::AdmissionContext,
    cache: &Path,
) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    context.reject_retained_path(cache)?;
    match std::fs::symlink_metadata(cache) {
        Ok(meta) => {
            if !meta.is_dir()
                || meta.uid() != unsafe { libc::geteuid() }
                || meta.mode() & 0o022 != 0
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "cache parent must be owned/protected; refusing chmod",
                ));
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut first = cache;
            while let Some(parent) = first.parent() {
                match std::fs::symlink_metadata(parent) {
                    Ok(_) => break,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => first = parent,
                    Err(e) => return Err(e),
                }
            }
            let setup = context.local(first)?;
            let mut create = setup.begin(crate::worker_admission::MutationPlan::Create {
                root: first.to_path_buf(),
            })?;
            create.create_dir_all(cache, 0o700)?;
            create.commit()?;
            Ok(())
        }
        Err(e) => Err(e),
    }
}

pub(crate) fn cache_candidates(cache: &Path, reference: &str) -> Vec<PathBuf> {
    let shared = cache.join(rootfs_slug_for_image(canonical_cache_key(reference)));
    let pinned = cache.join(rootfs_slug_for_image(reference));
    if shared == pinned {
        vec![shared]
    } else {
        vec![shared, pinned]
    }
}
fn lower_path(root: &Path) -> PathBuf {
    let mut name = root
        .file_name()
        .expect("cache entry filename")
        .to_os_string();
    name.push(".erofs");
    root.with_file_name(name)
}
trait CacheReads {
    fn exists(&self, path: &Path) -> std::io::Result<bool>;
    fn use_path(&self, path: &Path) -> std::io::Result<crate::worker_admission::UseGuard>;
}
impl CacheReads for crate::worker_admission::CacheProbe {
    fn exists(&self, p: &Path) -> std::io::Result<bool> {
        self.exists(p)
    }
    fn use_path(&self, p: &Path) -> std::io::Result<crate::worker_admission::UseGuard> {
        self.use_path(p)
    }
}
impl CacheReads for crate::worker_admission::CachePreparation {
    fn exists(&self, p: &Path) -> std::io::Result<bool> {
        self.exists(p)
    }
    fn use_path(&self, p: &Path) -> std::io::Result<crate::worker_admission::UseGuard> {
        self.use_path(p)
    }
}
struct CacheState {
    root: PathBuf,
    populated: bool,
    ready: bool,
    env: Vec<(String, String)>,
}
fn inspect_cache(
    reads: &impl CacheReads,
    reference: &str,
    roots: &[PathBuf],
) -> Result<CacheState> {
    fn inspect(reads: &impl CacheReads, root: &Path, pin: Option<&str>) -> Result<CacheState> {
        let mut state = CacheState {
            root: root.to_path_buf(),
            populated: false,
            ready: false,
            env: Vec::new(),
        };
        if !reads.exists(root)? {
            return Ok(state);
        }
        let source = reads.use_path(root)?;
        let meta = std::fs::metadata(source.path())?;
        anyhow::ensure!(meta.is_dir(), "cached source is not a directory");
        let bin = match std::fs::metadata(source.path().join("bin")) {
            Ok(meta) => meta.is_dir(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(e.into()),
        };
        let digest = digest_path(root);
        let (pin_matches, digest_time) = if reads.exists(&digest)? {
            let held = reads.use_path(&digest)?;
            let bytes = std::fs::read_to_string(held.path())?;
            let time = std::fs::metadata(held.path())?.modified()?;
            (
                pin.is_none_or(|pin| bytes.lines().any(|d| d.trim().eq_ignore_ascii_case(pin))),
                Some(time),
            )
        } else {
            (pin.is_none(), None)
        };
        // A populated directory without config cannot be a valid warm hit.
        let config = match crate::cli::worker_manager::oci::read_rootfs_file_no_follow(
            source.path(),
            Path::new(".oci-config.json"),
        ) {
            Ok(config) => config,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(state),
            Err(e) => return Err(e.into()),
        };
        let json: serde_json::Value = serde_json::from_slice(&config)?;
        anyhow::ensure!(json.is_object(), "OCI config is not an object");
        state.populated = bin && pin_matches;
        state.env =
            crate::cli::worker_manager::oci::oci_env_from_config(std::str::from_utf8(&config)?);
        let lower = lower_path(root);
        if state.populated && reads.exists(&lower)? {
            let held = reads.use_path(&lower)?;
            let lower_time = std::fs::metadata(held.path())?.modified()?;
            let config_time =
                std::fs::metadata(source.path().join(".oci-config.json"))?.modified()?;
            let fresh = lower_time >= meta.modified()?
                && lower_time >= config_time
                && digest_time.is_none_or(|time| lower_time >= time);
            if fresh {
                crate::cli::erofs::reader::ErofsReader::new(std::fs::File::open(held.path())?)?;
                state.ready = true;
            }
        }
        source.revalidate()?;
        Ok(state)
    }
    let pin = pinned_digest(reference);
    let mut selected = &roots[0];
    if roots.len() > 1 && pin.is_some() && reads.exists(&roots[0])? {
        let source = reads.use_path(&roots[0])?;
        let populated = match std::fs::metadata(source.path().join("bin")) {
            Ok(meta) => meta.is_dir(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(e.into()),
        };
        let digest = digest_path(&roots[0]);
        let matches = if reads.exists(&digest)? {
            let held = reads.use_path(&digest)?;
            std::fs::read_to_string(held.path())?
                .lines()
                .any(|d| d.trim().eq_ignore_ascii_case(pin.unwrap()))
        } else {
            false
        };
        if populated && !matches {
            selected = &roots[1];
        }
    }
    inspect(reads, selected, pin)
}

pub(crate) struct CacheRequest {
    consumer: crate::worker_admission::Lease,
    probe: crate::worker_admission::CacheProbe,
    reference: String,
    roots: Vec<PathBuf>,
}
impl CacheRequest {
    pub(crate) fn new(
        consumer: crate::worker_admission::Lease,
        reference: &str,
        cache: &Path,
    ) -> Result<Self> {
        let roots = cache_candidates(cache, reference);
        let entries: Vec<_> = roots
            .iter()
            .map(|root| crate::worker_admission::CacheEntry { root })
            .collect();
        let probe = consumer.probe_cache(&entries)?;
        Ok(Self {
            consumer,
            probe,
            reference: reference.to_string(),
            roots,
        })
    }
    /// Only the current, private retirement receipt can continue an existing
    /// lifecycle. The core binds and rechecks the SAME consumer Arc at handoff.
    pub(crate) fn from_retirement(
        consumer: crate::worker_admission::Lease,
        retirement: &crate::worker_admission::WriterRetirement,
        reference: &str,
        cache: &Path,
    ) -> Result<Self> {
        let roots = cache_candidates(cache, reference);
        let entries: Vec<_> = roots
            .iter()
            .map(|root| crate::worker_admission::CacheEntry { root })
            .collect();
        let probe = retirement.probe_cache(&consumer, &entries)?;
        Ok(Self {
            consumer,
            probe,
            reference: reference.to_string(),
            roots,
        })
    }

    pub(crate) async fn resolve(
        self,
        allow_pull: bool,
        on_pull: impl FnOnce() + Send + 'static,
    ) -> Result<Option<PreparedCache>> {
        self.resolve_with(
            allow_pull,
            on_pull,
            |reference, stage, checkpoint| async move {
                crate::cli::worker_manager::oci::pull_into_stage_checked(
                    &reference, &stage, checkpoint,
                )
                .await
            },
        )
        .await
    }

    // The producer boundary is the only substituted work in source-authored
    // cold fixtures. Admission, recheck, staging, EROFS and handoff stay real.
    async fn resolve_with<F, Fut>(
        self,
        allow_pull: bool,
        on_pull: impl FnOnce() + Send + 'static,
        pull: F,
    ) -> Result<Option<PreparedCache>>
    where
        F: FnOnce(String, PathBuf, Checkpoint) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<Vec<String>>> + Send + 'static,
    {
        // The task owns both capabilities through blocking/async work even if
        // the requesting future is dropped. No helper can issue a writer lease.
        tokio::spawn(async move {
            let state = inspect_cache(&self.probe, &self.reference, &self.roots)?;
            if state.ready {
                let lease = self.probe.handoff(&self.consumer, &state.root)?;
                return Ok(Some(PreparedCache {
                    lease,
                    reference: self.reference,
                    lower: lower_path(&state.root),
                    root: state.root,
                    env: state.env,
                }));
            }
            if !state.populated && !allow_pull {
                self.probe.cancel()?;
                return Ok(None);
            }
            let prep = self.probe.prepare()?;
            let mut state = inspect_cache(&prep, &self.reference, &self.roots)?;
            if !state.populated {
                anyhow::ensure!(allow_pull, "cache disappeared during exclusive recheck");
                on_pull();
                let mutation = MutationWriter::from_guard(prep.begin_entry(&state.root)?);
                let lock_path = state.root.with_file_name(format!(
                    "{}.lock",
                    state.root.file_name().unwrap().to_string_lossy()
                ));
                let diagnostic = mutation.diagnostic(&lock_path)?;
                let check = mutation.checkpoint.clone();
                let lock = tokio::task::spawn_blocking(move || {
                    MutationLock::acquire(diagnostic, check, true)
                })
                .await??;
                mutation.checkpoint.check()?;
                // Exact candidate selection was rechecked under exclusive prep;
                // raw external edits are caught by guard checkpoints after waits.
                let stage = mutation.apply(|g| g.stage_tree(&state.root))?;
                let digests = pull(
                    self.reference.clone(),
                    stage.path().to_path_buf(),
                    mutation.checkpoint.clone(),
                )
                .await?;
                let mut digest =
                    mutation.apply(|g| g.stage_file(&digest_path(&state.root), 0o600))?;
                use std::io::Write;
                digest.file_mut().write_all(digests.join("\n").as_bytes())?;
                digest.file_mut().sync_all()?;
                mutation.apply(|g| g.publish_tree(stage))?;
                mutation.apply(|g| g.publish_file(digest))?;
                lock.finish()?;
                mutation.commit()?;
                state = inspect_cache(&prep, &self.reference, &self.roots)?;
                anyhow::ensure!(
                    state.populated,
                    "pulled root lacks required population/digest/config"
                );
            }
            if !state.ready {
                let writer = prep.clone();
                let root = state.root.clone();
                tokio::task::spawn_blocking(move || {
                    crate::cli::erofs::build_prepared_erofs(&writer, &root)
                })
                .await??;
            }
            state = inspect_cache(&prep, &self.reference, &self.roots)?;
            anyhow::ensure!(state.ready, "prepared lower is not complete/fresh");
            let lease = prep.handoff(&self.consumer, &state.root)?;
            Ok(Some(PreparedCache {
                lease,
                reference: self.reference,
                lower: lower_path(&state.root),
                root: state.root,
                env: state.env,
            }))
        })
        .await
        .context("cache producer task failed")?
    }
    #[cfg(test)]
    pub(crate) async fn resolve_fixture_failure(self) -> Result<Option<PreparedCache>> {
        self.resolve_with(
            true,
            || {},
            |_, stage, checkpoint| async move {
                checkpoint.check()?;
                std::fs::write(stage.join("producer-evidence"), b"partial")?;
                anyhow::bail!("fixture producer failed after staging")
            },
        )
        .await
    }

    #[cfg(test)]
    pub(crate) async fn resolve_fixture(self, produce: bool) -> Result<Option<PreparedCache>> {
        self.resolve_with(
            produce,
            || {},
            |_, stage, checkpoint| async move {
                checkpoint.check()?;
                std::fs::create_dir(stage.join("bin"))?;
                std::fs::write(stage.join("bin/sh"), b"not executed")?;
                std::fs::write(
                    stage.join(".oci-config.json"),
                    br#"{"config":{"Env":["H1=fixture"]}}"#,
                )?;
                checkpoint.check()?;
                Ok(vec![format!("sha256:{}", "b".repeat(64))])
            },
        )
        .await
    }
}

fn record_digest(path: &Path, digests: &[String], checkpoint: &Checkpoint) -> Result<()> {
    // No clone may record a refusal between this check and publication.
    if let Err(e) = checkpoint.with_valid(|| std::fs::write(path, digests.join("\n")))? {
        tracing::warn!(path = %path.display(), error = %e, "failed to record the rootfs digest");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h_context() -> (
        tempfile::TempDir,
        crate::worker_admission::AdmissionContext,
        PathBuf,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let ns = temp.path().join("catalog");
        std::fs::create_dir(&ns).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ns, std::fs::Permissions::from_mode(0o700)).unwrap();
        let context = crate::worker_admission::testing::provision_context(&ns).unwrap();
        let cache = temp.path().join("cache");
        prepare_cache_parent(&context, &cache).unwrap();
        (temp, context, cache)
    }
    fn seed_h_root(
        context: &crate::worker_admission::AdmissionContext,
        owner: &Path,
        root: &Path,
        digest: &str,
    ) {
        use std::io::Write;
        let lease = context.local(owner).unwrap();
        let prep = lease
            .prepare_cache(&[crate::worker_admission::CacheEntry { root }])
            .unwrap();
        let mut guard = prep.begin_entry(root).unwrap();
        let tree = guard.stage_tree(root).unwrap();
        std::fs::create_dir(tree.path().join("bin")).unwrap();
        std::fs::write(tree.path().join("bin/sh"), b"fixture").unwrap();
        std::fs::write(
            tree.path().join(".oci-config.json"),
            br#"{"config":{"Env":["H=1"]}}"#,
        )
        .unwrap();
        let mut sidecar = guard.stage_file(&digest_path(root), 0o600).unwrap();
        sidecar.file_mut().write_all(digest.as_bytes()).unwrap();
        guard.publish_tree(tree).unwrap();
        guard.publish_file(sidecar).unwrap();
        guard.commit().unwrap();
        crate::cli::erofs::build_prepared_erofs(&prep, root).unwrap();
        prep.handoff(&lease, root).unwrap();
    }

    #[tokio::test]
    async fn h_first_and_second_warm_readers_share_without_diagnostic_or_inode_writes() {
        use std::os::unix::fs::MetadataExt;
        let (temp, context, cache) = h_context();
        let reference = "docker.io/library/alpine:3.20";
        let root = cache_candidates(&cache, reference).remove(0);
        seed_h_root(&context, &temp.path().join("seed"), &root, "sha256:fixture");
        let files = [
            root.clone(),
            lower_path(&root),
            digest_path(&root),
            PathBuf::from(format!("{}.lock", lower_path(&root).display())),
        ];
        let before: Vec<_> = files
            .iter()
            .map(|p| {
                let m = std::fs::metadata(p).unwrap();
                (m.ino(), m.mtime(), m.mtime_nsec(), m.len())
            })
            .collect();
        let a = CacheRequest::new(
            context.local(&temp.path().join("a")).unwrap(),
            reference,
            &cache,
        )
        .unwrap()
        .resolve(false, || panic!("warm pull"))
        .await
        .unwrap()
        .unwrap();
        let b = CacheRequest::new(
            context.local(&temp.path().join("b")).unwrap(),
            reference,
            &cache,
        )
        .unwrap()
        .resolve(false, || panic!("second warm pull"))
        .await
        .unwrap()
        .unwrap();
        assert_eq!(a.root, b.root);
        assert_eq!(b.env().unwrap(), vec![("H".into(), "1".into())]);
        let after: Vec<_> = files
            .iter()
            .map(|p| {
                let m = std::fs::metadata(p).unwrap();
                (m.ino(), m.mtime(), m.mtime_nsec(), m.len())
            })
            .collect();
        assert_eq!(before, after);
        assert!(!PathBuf::from(format!("{}.lock", root.display())).exists());
        let third = context.local(&temp.path().join("third")).unwrap();
        assert!(
            third
                .prepare_cache(&[crate::worker_admission::CacheEntry { root: &root }])
                .is_err()
        );
        let unrelated = cache.join("unrelated");
        assert!(
            third
                .prepare_cache(&[crate::worker_admission::CacheEntry { root: &unrelated }])
                .is_ok()
        );
        drop(a);
        b.validate(reference).unwrap();
    }

    #[tokio::test]
    async fn h_no_auto_install_missing_root_or_missing_pinned_digest_is_not_a_hit() {
        let (temp, context, cache) = h_context();
        let reference = "docker.io/library/alpine:3.20";
        let root = cache_candidates(&cache, reference).remove(0);
        assert!(
            CacheRequest::new(
                context.local(&temp.path().join("missing")).unwrap(),
                reference,
                &cache
            )
            .unwrap()
            .resolve(false, || panic!("no auto pull"))
            .await
            .unwrap()
            .is_none()
        );
        seed_h_root(&context, &temp.path().join("seed"), &root, "sha256:other");
        let pinned = format!("{reference}@sha256:{}", "a".repeat(64));
        assert!(
            CacheRequest::new(
                context.local(&temp.path().join("pin")).unwrap(),
                &pinned,
                &cache
            )
            .unwrap()
            .resolve(false, || panic!("pin mismatch pull"))
            .await
            .unwrap()
            .is_none()
        );
        assert!(!cache_candidates(&cache, &pinned)[1].exists());
    }

    #[test]
    fn h_surviving_stage_fd_blocks_completion_not_just_pathname_reopen() {
        use std::io::Write;
        let (temp, context, cache) = h_context();
        let root = cache.join("entry");
        let lease = context.local(&temp.path().join("consumer")).unwrap();
        let prep = lease
            .prepare_cache(&[crate::worker_admission::CacheEntry { root: &root }])
            .unwrap();
        let mut guard = prep.begin_entry(&root).unwrap();
        let mut stage = guard.stage_file(&digest_path(&root), 0o600).unwrap();
        stage.file_mut().write_all(b"evidence").unwrap();
        let clone = stage.file_mut().try_clone().unwrap();
        assert!(guard.publish_file(stage).is_err());
        assert!(prep.handoff(&lease, &root).is_err());
        drop(clone);
    }

    use tempfile::tempdir;

    #[test]
    fn first_refusal_is_shared_irreversible_and_retains_its_original_cause() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let checked = calls.clone();
        let checkpoint = Checkpoint::new(move || {
            if checked.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(std::io::Error::from_raw_os_error(libc::EIO))
            } else {
                Ok(())
            }
        });
        let a = checkpoint.clone();
        let b = checkpoint.clone();
        let (first, second) = std::thread::scope(|scope| {
            let a = scope.spawn(move || a.check().unwrap_err());
            let b = scope.spawn(move || b.check().unwrap_err());
            (a.join().unwrap(), b.join().unwrap())
        });
        assert!(Arc::ptr_eq(&first.0, &second.0));
        assert_eq!(first.0.raw_os_error(), Some(libc::EIO));
        assert_eq!(
            first.0.kind(),
            std::io::Error::from_raw_os_error(libc::EIO).kind()
        );
        let contextual = anyhow::Error::new(first)
            .context("stream chunk")
            .context("pull attempt");
        assert!(is_admission_refusal(&contextual));
        assert!(checkpoint.check().is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let fresh = Checkpoint::new(|| Ok(()));
        assert!(
            fresh.check().is_ok(),
            "independent operations have independent state"
        );
    }

    #[test]
    fn one_shot_refusal_prevents_later_digest_pid_and_cleanup() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let t = tempdir().unwrap();
        let path = t.path().join("held.lock");
        let calls = Arc::new(AtomicUsize::new(0));
        let checked = calls.clone();
        let checkpoint = Checkpoint::new(move || {
            if checked.fetch_add(1, Ordering::SeqCst) == 3 {
                Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "one-shot refusal",
                ))
            } else {
                Ok(())
            }
        });
        let lock = checked_cache_lock(&path, &checkpoint, true).unwrap();
        let diagnostic = std::fs::read(&path).unwrap();
        let other = checkpoint.clone();
        assert!(other.check().is_err());
        let digest = t.path().join("image.digest");
        assert!(record_digest(&digest, &["sha256:unused".into()], &checkpoint).is_err());
        let next_lock = t.path().join("never-created.lock");
        assert!(checked_cache_lock(&next_lock, &checkpoint, true).is_err());
        drop(lock);
        assert_eq!(std::fs::read(&path).unwrap(), diagnostic);
        assert!(!digest.exists());
        assert!(!next_lock.exists());
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        assert!(exclusive(&path));
    }

    #[test]
    fn one_shot_post_wait_refusal_preserves_long_diagnostic_and_releases_lock() {
        use std::os::fd::AsRawFd;
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let t = tempdir().unwrap();
        let path = t.path().join("image.lock");
        let prior = b"a stale diagnostic substantially longer than the next PID\n";
        std::fs::write(&path, prior).unwrap();
        let holder = std::fs::File::open(&path).unwrap();
        assert_eq!(unsafe { libc::flock(holder.as_raw_fd(), libc::LOCK_EX) }, 0);
        let calls = Arc::new(AtomicUsize::new(0));
        let checked = calls.clone();
        let (ready, waiting) = std::sync::mpsc::channel();
        let checkpoint = Checkpoint::new(move || {
            if checked.fetch_add(1, Ordering::SeqCst) == 2 {
                Err(std::io::Error::other("one-shot post-wait"))
            } else {
                Ok(())
            }
        })
        .on_contention(move || ready.send(()).unwrap());
        let worker = checkpoint.clone();
        let lock_path = path.clone();
        let task = std::thread::spawn(move || checked_cache_lock(&lock_path, &worker, true));
        waiting
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(!task.is_finished());
        drop(holder);
        assert!(task.join().unwrap().is_err());
        assert!(checkpoint.check().is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(std::fs::read(&path).unwrap(), prior);
        assert!(exclusive(&path));
    }

    #[test]
    fn explicit_lock_finish_propagates_one_shot_refusal_and_preserves_diagnostic() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        };
        let t = tempdir().unwrap();
        let path = t.path().join("image.lock");
        std::fs::write(&path, b"keep-diagnostic").unwrap();
        let refuse = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let trigger = refuse.clone();
        let seen = calls.clone();
        let checkpoint = Checkpoint::new(move || {
            seen.fetch_add(1, Ordering::SeqCst);
            if trigger.swap(false, Ordering::SeqCst) {
                Err(std::io::Error::other("one-shot cleanup refusal"))
            } else {
                Ok(())
            }
        });
        let lock = checked_cache_lock(&path, &checkpoint, false).unwrap();
        assert!(!exclusive(&path));
        refuse.store(true, Ordering::SeqCst);
        let error = lock.finish().unwrap_err();
        assert!(is_admission_refusal(&error));
        assert_eq!(std::fs::read(&path).unwrap(), b"keep-diagnostic");
        assert!(
            exclusive(&path),
            "finish must close the descriptor on refusal"
        );
        let after = calls.load(Ordering::SeqCst);
        assert!(checkpoint.check().is_err());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            after,
            "Drop/check must not retry the callback"
        );
        let fresh = Checkpoint::new(|| Ok(()));
        checked_cache_lock(&path, &fresh, false)
            .unwrap()
            .finish()
            .unwrap();
        assert!(std::fs::read(&path).unwrap().is_empty());
        assert!(exclusive(&path));
    }

    #[tokio::test]
    async fn locked_cache_hit_cannot_succeed_after_finish_refuses() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let t = tempdir().unwrap();
        let cache = t.path().join("cache");
        std::fs::create_dir(&cache).unwrap();
        let image = "docker.io/library/alpine:3.20";
        let dest = canonical_path_at(&cache, image);
        let worker_dest = dest.clone();
        let lock_path = lock_path_at(&cache, image);
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let checkpoint = Checkpoint::new(move || {
            match seen.fetch_add(1, Ordering::SeqCst) {
                // Simulate a prior pull having completed before the under-lock
                // cache-hit check, without touching global HOME or the network.
                5 => std::fs::create_dir_all(worker_dest.join("bin")),
                6 => Err(std::io::Error::other("cleanup-only refusal")),
                _ => Ok(()),
            }
        });
        let error = ensure_rootfs_at(
            image,
            || panic!("must hit under lock"),
            &cache,
            checkpoint.clone(),
            |_, _, _| async { panic!("must not pull") },
        )
        .await
        .unwrap_err();
        assert!(is_admission_refusal(&error));
        assert_eq!(
            std::fs::read_to_string(&lock_path).unwrap(),
            format!("{}\n", std::process::id())
        );
        assert!(dest.join("bin").is_dir());
        assert!(exclusive(&lock_path));
        assert!(checkpoint.check().is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 7);
    }

    #[tokio::test]
    async fn completed_pull_cannot_return_success_when_lock_finish_refuses() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let t = tempdir().unwrap();
        let cache = t.path().join("cache");
        std::fs::create_dir(&cache).unwrap();
        let image = "docker.io/library/alpine:3.20";
        let dest = canonical_path_at(&cache, image);
        let lock_path = lock_path_at(&cache, image);
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let checkpoint = Checkpoint::new(move || {
            if seen.fetch_add(1, Ordering::SeqCst) == 8 {
                Err(std::io::Error::other("cleanup-only refusal"))
            } else {
                Ok(())
            }
        });
        let next_effect = t.path().join("next-effect");
        let result = ensure_rootfs_at(
            image,
            || {},
            &cache,
            checkpoint.clone(),
            |_, dest, _| async move {
                std::fs::create_dir_all(dest.join("bin"))?;
                Ok(vec!["sha256:fixture".into()])
            },
        )
        .await;
        if result.is_ok() {
            std::fs::write(&next_effect, b"forbidden").unwrap();
        }
        assert!(is_admission_refusal(&result.unwrap_err()));
        assert!(!next_effect.exists());
        assert!(dest.join("bin").is_dir());
        assert_eq!(
            std::fs::read_to_string(digest_path(&dest)).unwrap(),
            "sha256:fixture"
        );
        assert_eq!(
            std::fs::read_to_string(&lock_path).unwrap(),
            format!("{}\n", std::process::id())
        );
        assert!(exclusive(&lock_path));
        assert!(checkpoint.check().is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 9);
    }

    #[test]
    fn diagnostic_effect_error_latches_without_callback_recovery() {
        let checkpoint = Checkpoint::new(|| Ok(()));
        let result: Result<(), AdmissionRefused> = checkpoint.with_io(|| {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "diagnostic write failed",
            ))
        });
        assert_eq!(result.unwrap_err().0.kind(), std::io::ErrorKind::BrokenPipe);
        assert!(checkpoint.clone().check().is_err());
        assert!(
            checkpoint
                .with_io(|| -> std::io::Result<()> {
                    panic!("must not retry cleanup after diagnostic failure")
                })
                .is_err()
        );
    }

    fn fence(t: &Path) -> (PathBuf, crate::worker_admission::Lease) {
        let owner = t.join("owner");
        std::fs::create_dir(&owner).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let catalog = t.join("catalog");
        std::fs::create_dir(&catalog).unwrap();
        std::fs::set_permissions(&catalog, std::fs::Permissions::from_mode(0o700)).unwrap();
        let context = crate::worker_admission::testing::provision_context(&catalog).unwrap();
        let lease = context.local(&owner.join("state")).unwrap();
        (owner, lease)
    }

    fn invalidate(owner: &Path) -> PathBuf {
        let old = owner.with_file_name("old-owner");
        std::fs::rename(owner, &old).unwrap();
        std::fs::create_dir(owner).unwrap();
        old
    }

    fn exclusive(path: &Path) -> bool {
        use std::os::fd::AsRawFd;
        let file = std::fs::File::open(path).unwrap();
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
    }

    #[test]
    fn checked_lock_preserves_pid_lifetime_and_pre_open_refusal() {
        let t = tempdir().unwrap();
        let path = t.path().join("image.lock");
        let refused: Checkpoint = Checkpoint::new(|| Err(std::io::Error::other("refused")));
        assert!(checked_cache_lock(&path, &refused, true).is_err());
        assert!(!path.exists());
        let old_diagnostic = b"older diagnostic much longer than the current process identifier\n";
        std::fs::write(&path, old_diagnostic).unwrap();
        let allowed: Checkpoint = Checkpoint::new(|| Ok(()));
        let lock = checked_cache_lock(&path, &allowed, true).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{}\n", std::process::id())
        );
        assert!(!exclusive(&path));
        drop(lock);
        assert!(std::fs::read(&path).unwrap().is_empty());
        assert!(exclusive(&path));
        // Compare the full-diagnostic fslock API through the existing dependency,
        // without reaching external source paths or executing this test now.
        std::fs::write(&path, old_diagnostic).unwrap();
        let mut legacy = fslock::LockFile::open(&path).unwrap();
        assert!(legacy.try_lock_with_pid().unwrap());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{}\n", std::process::id())
        );
        drop(legacy);
        assert!(std::fs::read(&path).unwrap().is_empty());
    }

    #[tokio::test]
    async fn checked_cache_hit_never_opens_a_lock_or_calls_pull() {
        let t = tempdir().unwrap();
        let image = "docker.io/library/alpine:3.20";
        let dest = canonical_path_at(t.path(), image);
        std::fs::create_dir_all(dest.join("bin")).unwrap();
        let checkpoint: Checkpoint = Checkpoint::new(|| Ok(()));
        let got = ensure_rootfs_at(
            image,
            || panic!("cache hit"),
            t.path(),
            checkpoint,
            |_, _, _| async { panic!("cache hit must not pull") },
        )
        .await
        .unwrap();
        assert_eq!(got, dest);
        assert!(!lock_path_at(t.path(), image).exists());
    }

    #[tokio::test]
    async fn invalid_admission_precedes_parent_and_lockfile_creation() {
        for missing_parent in [false, true] {
            let t = tempdir().unwrap();
            let (owner, lease) = fence(t.path());
            let old = invalidate(&owner);
            let cache = t.path().join("cache");
            if !missing_parent {
                std::fs::create_dir(&cache).unwrap();
            }
            let checkpoint: Checkpoint = Checkpoint::new(move || lease.revalidate());
            assert!(
                lock_at_path(cache.join("image.lock"), checkpoint)
                    .await
                    .is_err()
            );
            assert_eq!(cache.exists(), !missing_parent);
            assert!(!cache.join("image.lock").exists());
            assert!(exclusive(&old), "refusal must release the retained fence");
        }
    }

    #[tokio::test]
    async fn cache_rechecks_inside_lock_before_pid_removal_or_pull() {
        use std::os::fd::AsRawFd;
        use std::sync::Mutex;
        let t = tempdir().unwrap();
        let (owner, lease) = fence(t.path());
        let cache = t.path().join("cache");
        std::fs::create_dir(&cache).unwrap();
        let image = "docker.io/library/alpine:3.20";
        let dest = canonical_path_at(&cache, image);
        std::fs::create_dir(&dest).unwrap();
        std::fs::write(dest.join("sentinel"), b"keep-root").unwrap();
        let lower = dest.with_file_name(format!(
            "{}.erofs",
            dest.file_name().unwrap().to_str().unwrap()
        ));
        std::fs::write(&lower, b"keep-erofs").unwrap();
        std::fs::write(digest_path(&dest), b"keep-digest").unwrap();
        let lock_path = lock_path_at(&cache, image);
        std::fs::write(&lock_path, b"keep-lock").unwrap();
        let holder = std::fs::File::open(&lock_path).unwrap();
        assert_eq!(unsafe { libc::flock(holder.as_raw_fd(), libc::LOCK_EX) }, 0);
        let (ready, waiting) = tokio::sync::oneshot::channel();
        let ready = Mutex::new(Some(ready));
        let checkpoint = Checkpoint::new(move || lease.revalidate()).on_contention(move || {
            ready.lock().unwrap().take().unwrap().send(()).unwrap();
        });
        let worker_cache = cache.clone();
        let task = tokio::spawn(async move {
            ensure_rootfs_at(
                image,
                || panic!("no pull note"),
                &worker_cache,
                checkpoint,
                |_, _, _| async { panic!("no network/pull") },
            )
            .await
        });
        waiting.await.unwrap();
        assert!(!task.is_finished(), "the other holder still owns the lock");
        let old = invalidate(&owner);
        drop(holder);
        assert!(task.await.unwrap().is_err());
        assert_eq!(std::fs::read(dest.join("sentinel")).unwrap(), b"keep-root");
        assert_eq!(std::fs::read(lower).unwrap(), b"keep-erofs");
        assert_eq!(std::fs::read(digest_path(&dest)).unwrap(), b"keep-digest");
        assert_eq!(std::fs::read(lock_path).unwrap(), b"keep-lock");
        assert!(exclusive(&old));
    }

    #[tokio::test]
    async fn completed_pull_cannot_publish_digest_after_wait_invalidates_fence() {
        let t = tempdir().unwrap();
        let (owner, lease) = fence(t.path());
        let cache = t.path().join("cache");
        std::fs::create_dir(&cache).unwrap();
        let image = "docker.io/library/alpine:3.20";
        let dest = canonical_path_at(&cache, image);
        let checkpoint: Checkpoint = Checkpoint::new(move || lease.revalidate());
        let (ready, waiting) = tokio::sync::oneshot::channel();
        let (resume, resumed) = tokio::sync::oneshot::channel();
        let worker_cache = cache.clone();
        let task = tokio::spawn(async move {
            ensure_rootfs_at(
                image,
                || {},
                &worker_cache,
                checkpoint,
                |_, _, _| async move {
                    ready.send(()).unwrap();
                    resumed.await.unwrap();
                    Ok(vec!["sha256:fixture".into()])
                },
            )
            .await
        });
        waiting.await.unwrap();
        let old = invalidate(&owner);
        resume.send(()).unwrap();
        assert!(task.await.unwrap().is_err());
        assert!(!digest_path(&dest).exists());
        assert!(!dest.exists());
        assert!(exclusive(&old));
    }

    fn populate(dir: &Path) {
        std::fs::create_dir_all(dir.join("bin")).unwrap();
    }

    #[test]
    fn canonical_path_is_slug_based_and_stable() {
        let _g = crate::cli::test_support::lock_home();
        let a = canonical_path("docker.io/library/python:3.13");
        let b = canonical_path("docker.io/library/python:3.13");
        assert_eq!(a, b);
        assert_ne!(a, canonical_path("docker.io/library/python:3.12"));
    }

    /// A pin shares the tag's dir only while that dir was pulled from the
    /// pinned digest; changing the pin used to keep serving the old rootfs.
    #[test]
    fn a_pin_shares_the_tag_dir_only_while_its_digest_matches() {
        let _g = crate::cli::test_support::lock_home();
        let home = tempdir().unwrap();
        let prior = std::env::var_os("HOME");
        // SAFETY: the caller holds TEST_HOME_LOCK, which serializes all env mutation.
        unsafe { std::env::set_var("HOME", home.path()) };

        let tag = "docker.io/iiidev/node:latest";
        let (index, platform, other) = (
            format!("sha256:{}", "a".repeat(64)),
            format!("sha256:{}", "b".repeat(64)),
            format!("sha256:{}", "c".repeat(64)),
        );
        let shared = canonical_path(tag);
        let pinned_before_pull = canonical_path(&format!("{tag}@{index}"));
        populate(&shared);
        std::fs::write(digest_path(&shared), format!("{index}\n{platform}")).unwrap();
        let hit_index = resolve_cached(&format!("{tag}@{index}"));
        let hit_platform = resolve_cached(&format!("{tag}@{platform}"));
        let hit_other = resolve_cached(&format!("{tag}@{other}"));
        let other_path = canonical_path(&format!("{tag}@{other}"));

        match prior {
            Some(v) => unsafe { std::env::set_var("HOME", v) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        assert_eq!(hit_index.as_ref(), Some(&shared));
        assert_eq!(hit_platform.as_ref(), Some(&shared));
        assert_eq!(
            hit_other, None,
            "a changed pin was served another digest's rootfs"
        );
        assert_ne!(other_path, shared);
        assert_eq!(
            pinned_before_pull, shared,
            "a pin's first pull fills the tag's empty dir"
        );
        assert_eq!(
            lock_path_for_image(tag),
            lock_path_for_image(&format!("{tag}@{other}"))
        );
    }

    /// A digest-only pin pulled first used to fill `repo:latest`'s empty dir,
    /// so the tag was then served that digest whatever `latest` pointed to.
    #[test]
    fn a_digest_only_pin_never_fills_the_latest_dir() {
        let _g = crate::cli::test_support::lock_home();
        let home = tempdir().unwrap();
        let prior = std::env::var_os("HOME");
        // SAFETY: the caller holds TEST_HOME_LOCK, which serializes all env mutation.
        unsafe { std::env::set_var("HOME", home.path()) };

        let digest = format!("sha256:{}", "d".repeat(64));
        let pinned = format!("docker.io/iiidev/node@{digest}");
        let pin_dir = canonical_path(&pinned);
        populate(&pin_dir);
        std::fs::write(digest_path(&pin_dir), &digest).unwrap();
        let pin_hit = resolve_cached(&pinned);
        let latest_hit = resolve_cached("docker.io/iiidev/node:latest");
        let bare_hit = resolve_cached("docker.io/iiidev/node");

        match prior {
            Some(v) => unsafe { std::env::set_var("HOME", v) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        assert_eq!(pin_hit, Some(pin_dir));
        assert_eq!(
            latest_hit, None,
            "the tag was served a digest-only pin's rootfs"
        );
        assert_eq!(bare_hit, None);
    }

    #[test]
    fn the_cached_digest_is_the_one_the_reference_resolved_to() {
        let _g = crate::cli::test_support::lock_home();
        let home = tempdir().unwrap();
        let prior = std::env::var_os("HOME");
        // SAFETY: the caller holds TEST_HOME_LOCK, which serializes all env mutation.
        unsafe { std::env::set_var("HOME", home.path()) };
        let tag = "docker.io/iiidev/node:latest";
        let missing = cached_digest(tag);
        let rootfs = canonical_path(tag);
        populate(&rootfs);
        let (index, platform) = (
            format!("sha256:{}", "a".repeat(64)),
            format!("sha256:{}", "b".repeat(64)),
        );
        std::fs::write(digest_path(&rootfs), format!("{index}\n{platform}")).unwrap();
        let got = cached_digest(tag);
        match prior {
            Some(v) => unsafe { std::env::set_var("HOME", v) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        assert_eq!(missing, None);
        assert_eq!(got, Some(index));
    }

    #[test]
    fn is_populated_requires_bin_subdir() {
        let td = tempdir().unwrap();
        assert!(!is_populated(td.path()));
        populate(td.path());
        assert!(is_populated(td.path()));
    }

    #[test]
    fn resolve_cached_returns_none_on_total_miss() {
        let _g = crate::cli::test_support::lock_home();
        // Use a slug nothing on disk will have. Canonical_path respects
        // $HOME, so we don't need to redirect that — just pick an image
        // reference that will hash to a unique slug per test run.
        let uniq = format!(
            "nonexistent-{}:{}",
            std::process::id(),
            std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos()
        );
        assert!(resolve_cached(&uniq).is_none());
    }

    /// The lock is what stops a prefetch and the spawn it was warming from
    /// deleting each other's half-extracted rootfs. Two holders of the same
    /// image's lock must not overlap; two different images must not block.
    #[tokio::test]
    async fn the_pull_lock_excludes_one_image_and_not_another() {
        let tmp = tempdir().unwrap();
        let lock_a = tmp.path().join("a.lock");
        let lock_b = tmp.path().join("b.lock");

        let mut first = fslock::LockFile::open(&lock_a).unwrap();
        first.lock_with_pid().unwrap();

        // A second holder of the SAME lock waits. `try_lock` is the
        // non-blocking form of what `lock_for_image` does, so a refusal here
        // is exactly the wait a real second caller would take.
        let mut same = fslock::LockFile::open(&lock_a).unwrap();
        assert!(
            !same.try_lock_with_pid().unwrap(),
            "a second caller took the same image's lock while it was held"
        );

        // A different image is unrelated and must not be serialized behind it.
        let mut other = fslock::LockFile::open(&lock_b).unwrap();
        assert!(
            other.try_lock_with_pid().unwrap(),
            "a different image's pull was blocked by an unrelated lock"
        );

        drop(first);
        let mut after = fslock::LockFile::open(&lock_a).unwrap();
        assert!(
            after.try_lock_with_pid().unwrap(),
            "the lock was not released when its holder dropped"
        );
    }

    #[test]
    fn the_lock_file_sits_beside_the_rootfs_it_guards() {
        let _g = crate::cli::test_support::lock_home();
        let dest = canonical_path("docker.io/iiidev/node:latest");
        let lock = lock_path_for_image("docker.io/iiidev/node:latest");
        // Beside, never inside: the pull begins by deleting `dest`.
        assert_eq!(lock.parent(), dest.parent());
        assert_ne!(lock, dest);
        assert!(!lock.starts_with(&dest));
        // Two references to the same image share one lock; two images do not.
        assert_eq!(lock, lock_path_for_image("iiidev/node:latest"));
        assert_ne!(lock, lock_path_for_image("docker.io/iiidev/python:latest"));
    }
}
