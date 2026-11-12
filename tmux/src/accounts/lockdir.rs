//! Claude Code's own locks, taken the way Claude Code takes them
//! (agent-accounts G3 step 1): `proper-lockfile` directories. A lock is an
//! empty directory made with `mkdir`; one that exists is held, unless its
//! modification time is older than the lock's stale period, when it is
//! removed (`rmdir`) and taken again. Holders touch it to stay fresh; the
//! worker holds its locks for well under a second and touches them between
//! steps. Release is `rmdir`, in the reverse order of taking.
//!
//! Claude Code 2.1.322 can also take over `.oauth_refresh.lock` from a dead
//! holder, but only through an owner record
//! (`.oauth_refresh.lock.owner`, `{pid, procStart, pidDomain, pidSpace,
//! lockBirthtimeMs}`) whose `lockBirthtimeMs` is the lock directory's own
//! birth time and whose holder is proven gone; without a record
//! (`no_owner_record`) it never takes over. The worker writes no record, so
//! its live lock is never taken over, and it never touches a record Claude
//! Code wrote (a record left by a dead process names another directory's
//! birth time, so it can't match the worker's new directory either).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

/// `<config>/.oauth_refresh.lock`: Claude Code's refresh lock.
pub const REFRESH: &str = ".oauth_refresh.lock";
pub const REFRESH_STALE: Duration = Duration::from_secs(60);
/// `<config>/.storage-write.lock`: Claude Code's sign-in store lock.
pub const STORAGE: &str = ".storage-write.lock";
pub const STORAGE_STALE: Duration = Duration::from_secs(15);
/// `realpath(<config>) + ".lock"`: the refresh lock of older versions.
pub const LEGACY_STALE: Duration = Duration::from_secs(60);
/// `<file>.lock` of a `.claude.json`.
pub const CONFIG_STALE: Duration = Duration::from_secs(10);
/// How long the worker waits for all the locks of a swap.
pub const WAIT: Duration = Duration::from_secs(10);

/// Why a lock was not taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockError {
    /// Someone holds it (Claude Code renewing its sign-in, say).
    Busy,
    /// The file system refused.
    Io(String),
}

impl LockError {
    pub fn sentence(&self) -> String {
        match self {
            Self::Busy => super::slot::BUSY_TEXT.to_string(),
            Self::Io(error) => error.clone(),
        }
    }
}

/// A lock directory as it was when taken: what a recovery after a crash
/// recognises as the worker's own (G7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockId {
    pub path: PathBuf,
    pub dev: u64,
    pub ino: u64,
    pub mtime_ms: u64,
}

fn mtime_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|at| at.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|at| at.as_millis() as u64)
        .unwrap_or(0)
}

fn identity(path: &Path) -> Option<LockId> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path).ok()?;
    meta.is_dir().then(|| LockId {
        path: path.to_path_buf(),
        dev: meta.dev(),
        ino: meta.ino(),
        mtime_ms: mtime_ms(&meta),
    })
}

/// The legacy lock of config folder `dir`: `realpath(dir) + ".lock"`
/// (`dir` itself when it does not resolve, as Claude Code does).
pub fn legacy_path(dir: &Path) -> PathBuf {
    let real = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let mut text = real.into_os_string();
    text.push(".lock");
    PathBuf::from(text)
}

/// The lock of a `.claude.json`: `<file>.lock`.
pub fn config_lock_path(file: &Path) -> PathBuf {
    let mut text = file.as_os_str().to_owned();
    text.push(".lock");
    PathBuf::from(text)
}

/// Locks taken in order, released in reverse (also when dropped).
#[derive(Debug, Default)]
pub struct LockSet {
    held: Vec<LockId>,
}

impl LockSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take the lock at `path` (a directory that `mkdir` makes), waiting
    /// until `deadline` for a holder whose lock is younger than `stale`.
    pub fn take(
        &mut self,
        path: &Path,
        stale: Duration,
        deadline: Instant,
    ) -> Result<(), LockError> {
        let mut pause = Duration::from_millis(20);
        loop {
            match std::fs::create_dir(path) {
                Ok(()) => {
                    let id = identity(path).ok_or_else(|| {
                        LockError::Io(format!("The lock {} vanished.", path.display()))
                    })?;
                    self.held.push(id);
                    return Ok(());
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    match std::fs::symlink_metadata(path) {
                        Ok(meta) => {
                            let age = SystemTime::now()
                                .duration_since(meta.modified().unwrap_or(SystemTime::now()))
                                .unwrap_or_default();
                            if age > stale {
                                // proper-lockfile's own rule: a lock not
                                // touched within its stale period is gone.
                                let _ = std::fs::remove_dir(path);
                                continue;
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(error) => {
                            return Err(LockError::Io(format!(
                                "Could not look at {} ({error}).",
                                path.display()
                            )))
                        }
                    }
                    if Instant::now() >= deadline {
                        return Err(LockError::Busy);
                    }
                    std::thread::sleep(
                        pause.min(deadline.saturating_duration_since(Instant::now())),
                    );
                    pause = (pause * 2).min(Duration::from_millis(200));
                }
                Err(error) => {
                    return Err(LockError::Io(format!(
                        "Could not take {} ({error}).",
                        path.display()
                    )))
                }
            }
        }
    }

    /// The locks held, in the order taken.
    pub fn ids(&self) -> Vec<LockId> {
        self.held.clone()
    }

    pub fn holds(&self, path: &Path) -> bool {
        self.held.iter().any(|id| id.path == path)
    }

    /// Touch every lock held, as proper-lockfile's holders do, so none
    /// grows stale while the worker holds it.
    pub fn touch(&mut self) {
        let now = filetime_now();
        for id in &mut self.held {
            if set_mtime(&id.path, now).is_ok() {
                if let Some(fresh) = identity(&id.path) {
                    id.mtime_ms = fresh.mtime_ms;
                }
            }
        }
    }

    /// Release `path` and every lock taken after it, newest first.
    pub fn release_from(&mut self, path: &Path) {
        if let Some(at) = self.held.iter().position(|id| id.path == path) {
            while self.held.len() > at {
                if let Some(id) = self.held.pop() {
                    release(&id);
                }
            }
        }
    }

    /// Release them all, newest first.
    pub fn release_all(&mut self) {
        while let Some(id) = self.held.pop() {
            release(&id);
        }
    }
}

impl Drop for LockSet {
    fn drop(&mut self) {
        self.release_all();
    }
}

/// `rmdir`, only when the directory is still the one taken: a lock removed
/// as stale and taken by someone else since is theirs.
fn release(id: &LockId) {
    match identity(&id.path) {
        Some(now) if now.dev == id.dev && now.ino == id.ino => {
            if let Err(error) = std::fs::remove_dir(&id.path) {
                tracing::warn!(lock = %id.path.display(), %error, "lock not released");
            }
        }
        _ => tracing::warn!(lock = %id.path.display(), "lock taken over before its release"),
    }
}

/// Remove the locks a crashed swap of this worker left (G7): each still
/// the very directory it took (same device, inode and modification time).
/// Anything else is someone's lock and stays.
pub fn reclaim(ids: &[LockId]) {
    for id in ids.iter().rev() {
        if let Some(now) = identity(&id.path) {
            if now.dev == id.dev && now.ino == id.ino && now.mtime_ms == id.mtime_ms {
                let _ = std::fs::remove_dir(&id.path);
            }
        }
    }
}

fn filetime_now() -> SystemTime {
    SystemTime::now()
}

fn set_mtime(path: &Path, at: SystemTime) -> std::io::Result<()> {
    // Directories open read-only for `futimens`.
    let dir = std::fs::File::open(path)?;
    dir.set_times(std::fs::FileTimes::new().set_modified(at).set_accessed(at))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tmux-lockdir-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn age(path: &Path, by: Duration) {
        let at = SystemTime::now() - by;
        set_mtime(path, at).unwrap();
    }

    #[test]
    fn a_free_lock_is_taken_and_released_in_reverse() {
        let dir = temp("free");
        let (a, b) = (dir.join("a.lock"), dir.join("b.lock"));
        let mut set = LockSet::new();
        let deadline = Instant::now() + Duration::from_secs(1);
        set.take(&a, REFRESH_STALE, deadline).unwrap();
        set.take(&b, STORAGE_STALE, deadline).unwrap();
        assert!(a.is_dir() && b.is_dir());
        assert!(std::fs::read_dir(&a).unwrap().next().is_none(), "empty");
        assert_eq!(set.ids().len(), 2);
        set.release_from(&b);
        assert!(a.is_dir() && !b.exists());
        drop(set);
        assert!(!a.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_held_lock_is_busy_and_a_stale_one_is_taken() {
        let dir = temp("held");
        let lock = dir.join(".storage-write.lock");
        // Another holder (Claude Code, with proper-lockfile) made it now.
        std::fs::create_dir(&lock).unwrap();
        let mut set = LockSet::new();
        let started = Instant::now();
        let busy = set.take(
            &lock,
            STORAGE_STALE,
            Instant::now() + Duration::from_millis(300),
        );
        assert_eq!(busy, Err(LockError::Busy));
        assert!(started.elapsed() >= Duration::from_millis(300));
        assert!(lock.is_dir(), "theirs, untouched");
        // Not touched for longer than its stale period: gone.
        age(&lock, STORAGE_STALE + Duration::from_secs(1));
        set.take(
            &lock,
            STORAGE_STALE,
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        assert!(set.holds(&lock));
        drop(set);
        assert!(!lock.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_holder_that_lets_go_is_waited_for() {
        let dir = temp("wait");
        let lock = dir.join(".oauth_refresh.lock");
        std::fs::create_dir(&lock).unwrap();
        let theirs = lock.clone();
        let holder = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            std::fs::remove_dir(&theirs).unwrap();
        });
        let mut set = LockSet::new();
        set.take(
            &lock,
            REFRESH_STALE,
            Instant::now() + Duration::from_secs(2),
        )
        .unwrap();
        holder.join().unwrap();
        drop(set);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn touching_keeps_a_lock_fresh_and_release_spares_a_lock_taken_over() {
        let dir = temp("touch");
        let lock = dir.join("x.lock");
        let mut set = LockSet::new();
        set.take(&lock, CONFIG_STALE, Instant::now() + Duration::from_secs(1))
            .unwrap();
        age(&lock, Duration::from_secs(30));
        set.touch();
        let meta = std::fs::metadata(&lock).unwrap();
        let age_now = SystemTime::now()
            .duration_since(meta.modified().unwrap())
            .unwrap_or_default();
        assert!(age_now < Duration::from_secs(5));
        // Someone removed it as stale and took it: it is theirs now.
        std::fs::remove_dir(&lock).unwrap();
        std::fs::create_dir(dir.join("other")).unwrap();
        std::fs::create_dir(&lock).unwrap();
        drop(set);
        assert!(lock.is_dir(), "their lock stays");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_crashed_swap_s_locks_are_reclaimed_and_nobody_else_s() {
        let dir = temp("reclaim");
        let (ours, theirs) = (dir.join("ours.lock"), dir.join("theirs.lock"));
        let mut set = LockSet::new();
        let deadline = Instant::now() + Duration::from_secs(1);
        set.take(&ours, REFRESH_STALE, deadline).unwrap();
        set.take(&theirs, REFRESH_STALE, deadline).unwrap();
        let ids = set.ids();
        // The worker crashed: the locks stay as they were.
        std::mem::forget(set);
        // `theirs` was taken over (removed and made again) since.
        std::fs::remove_dir(&theirs).unwrap();
        std::fs::create_dir(dir.join("spacer")).unwrap();
        std::fs::create_dir(&theirs).unwrap();
        age(&theirs, Duration::from_secs(1));
        reclaim(&ids);
        assert!(!ours.exists());
        assert!(theirs.is_dir());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn legacy_and_config_lock_paths_follow_claude_code() {
        let dir = temp("legacy");
        let config = dir.join("home/.claude");
        std::fs::create_dir_all(&config).unwrap();
        let link = dir.join("link");
        std::os::unix::fs::symlink(&config, &link).unwrap();
        let real = std::fs::canonicalize(&config).unwrap();
        assert_eq!(legacy_path(&link), real.with_file_name(".claude.lock"));
        assert_eq!(
            legacy_path(Path::new("/nowhere/x")),
            Path::new("/nowhere/x.lock")
        );
        assert_eq!(
            config_lock_path(Path::new("/h/.claude.json")),
            Path::new("/h/.claude.json.lock")
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
