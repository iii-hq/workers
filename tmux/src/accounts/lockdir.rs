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
    /// When last noted; the worker touches it after that too.
    pub mtime_ms: u64,
    /// When the directory was made, in nanoseconds, where the file system
    /// keeps it (macOS, and Linux through `statx` on most file systems).
    #[serde(default)]
    pub birth_ns: Option<u64>,
}

/// How long after the journal last noted a lock the worker may still touch
/// it: a swap stops once it has held its locks for 8 seconds (the slot's
/// `HOLD_BUDGET`). A directory someone made in its place once it went
/// stale, in the same inode or not, is younger than the worker's last
/// touch by more than the lock's stale period: 15 s at the least for the
/// locks a journal notes (the store lock), so 5 s clear of this window.
/// One of the worker's touched later than that (a roll back past the
/// budget) is waited out as stale instead.
const OWN_TOUCHES: Duration = Duration::from_secs(10);

fn since_epoch(at: std::io::Result<SystemTime>) -> Option<Duration> {
    at.ok()
        .and_then(|at| at.duration_since(SystemTime::UNIX_EPOCH).ok())
}

fn identity(path: &Path) -> Option<LockId> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    meta.is_dir().then(|| id_of(path, &meta))
}

fn id_of(path: &Path, meta: &std::fs::Metadata) -> LockId {
    use std::os::unix::fs::MetadataExt;
    LockId {
        path: path.to_path_buf(),
        dev: meta.dev(),
        ino: meta.ino(),
        mtime_ms: since_epoch(meta.modified()).map_or(0, |at| at.as_millis() as u64),
        birth_ns: since_epoch(meta.created()).map(|at| at.as_nanos() as u64),
    }
}

/// Whether `now` is still the very directory `then` was, untouched since.
/// The device and inode alone don't tell: a file system may hand a freed
/// inode number out again (ext4 often does, at once), so a lock removed
/// and made again in the same place can have its predecessor's. Its
/// modification time is new, though (a lock is judged stale, or noted as
/// held, on the old one), and so is its birth time where the file system
/// keeps one. Renaming a directory within its folder changes neither.
fn same_dir(then: &LockId, now: &LockId) -> bool {
    now.dev == then.dev
        && now.ino == then.ino
        && now.mtime_ms == then.mtime_ms
        && match (then.birth_ns, now.birth_ns) {
            (Some(then), Some(now)) => then == now,
            _ => true,
        }
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
        sweep_aside(path, stale);
        let mut pause = Duration::from_millis(20);
        // Why a stale lock could not be removed, when it could not.
        let mut stuck: Option<String> = None;
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
                                match remove_stale(path, &id_of(path, &meta)) {
                                    Ok(()) => continue,
                                    // Waited for like a held lock, so a
                                    // folder that refuses never spins.
                                    Err(why) => stuck = Some(why),
                                }
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
                        return Err(match stuck {
                            Some(why) => LockError::Io(why),
                            None => LockError::Busy,
                        });
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

/// Remove the stale lock at `path` (`seen` as it was looked at). It is
/// first renamed to a name of its own, so a lock someone took between the
/// look and the removal is never the one removed: when the directory
/// renamed is not the one looked at ([`same_dir`]: a new lock, maybe in
/// the same inode, or the old one touched by its holder meanwhile), it
/// goes back.
fn remove_stale(path: &Path, seen: &LockId) -> Result<(), String> {
    let mut aside = path.as_os_str().to_owned();
    aside.push(format!(".stale-{}", uuid::Uuid::new_v4().simple()));
    let aside = PathBuf::from(aside);
    match std::fs::rename(path, &aside) {
        Ok(()) => {}
        // Someone removed it first: free to take.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "Could not remove the stale lock {} ({error}).",
                path.display()
            ))
        }
    }
    let same =
        std::fs::symlink_metadata(&aside).is_ok_and(|now| same_dir(seen, &id_of(&aside, &now)));
    if !same {
        // Theirs, taken meanwhile: back where it was, unless a new one is
        // there already (never over it). Left aside, it goes once stale.
        if let Err(error) = rename_no_replace(&aside, path) {
            tracing::warn!(lock = %path.display(), %error, "a lock moved aside was not put back");
        }
        return Ok(());
    }
    if let Err(error) = std::fs::remove_dir(&aside) {
        tracing::warn!(lock = %aside.display(), %error, "stale lock moved aside but not removed");
    }
    Ok(())
}

/// `rename(from, to)` that fails when `to` exists, in one call: Linux's
/// `renameat2(RENAME_NOREPLACE)`, macOS's `renamex_np(RENAME_EXCL)`. Where
/// the file system can't (an old kernel, some network file systems), a
/// look then a plain rename, which a directory made in between would lose
/// to.
fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = |path: &Path| {
        std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))
    };
    let (from_c, to_c) = (c(from)?, c(to)?);
    #[cfg(target_os = "linux")]
    // SAFETY: both paths are NUL-terminated strings that outlive the call.
    let done = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            from_c.as_ptr(),
            libc::AT_FDCWD,
            to_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        ) as libc::c_int
    };
    #[cfg(target_os = "macos")]
    // SAFETY: both paths are NUL-terminated strings that outlive the call.
    let done = unsafe { libc::renamex_np(from_c.as_ptr(), to_c.as_ptr(), libc::RENAME_EXCL) };
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let done = {
        let _ = (&from_c, &to_c);
        -1
    };
    if done == 0 {
        return Ok(());
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let error = std::io::Error::last_os_error();
        let unsupported = matches!(
            error.raw_os_error(),
            Some(libc::ENOSYS) | Some(libc::EINVAL) | Some(libc::ENOTSUP)
        );
        if !unsupported {
            return Err(error);
        }
    }
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(std::io::ErrorKind::AlreadyExists.into());
    }
    std::fs::rename(from, to)
}

/// Remove the directories [`remove_stale`] moved aside from `lock`
/// (`<lock>.stale-<32 hex>`) that are past `stale`: one a crash left
/// between the rename and the removal, or someone's lock that could not be
/// put back. Each `rmdir` fails on anything not empty.
pub fn sweep_aside(lock: &Path, stale: Duration) {
    let (Some(parent), Some(name)) = (lock.parent(), lock.file_name()) else {
        return;
    };
    let mut prefix = name.to_os_string();
    prefix.push(".stale-");
    let prefix = prefix.to_string_lossy().into_owned();
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(tag) = name.strip_prefix(&prefix) else {
            continue;
        };
        if tag.len() != 32 || !tag.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let age = SystemTime::now()
            .duration_since(meta.modified().unwrap_or(SystemTime::now()))
            .unwrap_or_default();
        if meta.is_dir() && age > stale {
            let _ = std::fs::remove_dir(&path);
        }
    }
}

/// `rmdir`, only when the directory is still the one taken and last
/// touched ([`same_dir`]): a lock removed as stale and taken by someone
/// else since is theirs, in the same inode or not.
fn release(id: &LockId) {
    match identity(&id.path) {
        Some(now) if same_dir(id, &now) => {
            if let Err(error) = std::fs::remove_dir(&id.path) {
                tracing::warn!(lock = %id.path.display(), %error, "lock not released");
            }
        }
        _ => tracing::warn!(lock = %id.path.display(), "lock taken over before its release"),
    }
}

/// Remove the locks a crashed swap of this worker left (G7): each still
/// the very directory it took. That is the same device and inode, and the
/// same birth time where the file system keeps one; where it doesn't, a
/// modification time within the worker's own touches after the journal
/// noted it ([`OWN_TOUCHES`]), which a directory someone made in its place
/// (an inode used again) can't have. Anything else is someone's lock and
/// stays; one of the worker's not recognised is waited out as stale.
pub fn reclaim(ids: &[LockId]) {
    for id in ids.iter().rev() {
        if let Some(now) = identity(&id.path) {
            if is_own(id, &now) {
                let _ = std::fs::remove_dir(&id.path);
            }
        }
    }
}

fn is_own(noted: &LockId, now: &LockId) -> bool {
    if now.dev != noted.dev || now.ino != noted.ino {
        return false;
    }
    match (noted.birth_ns, now.birth_ns) {
        (Some(then), Some(now)) => then == now,
        _ => {
            now.mtime_ms >= noted.mtime_ms
                && now.mtime_ms < noted.mtime_ms + OWN_TOUCHES.as_millis() as u64
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
    fn a_stale_lock_that_cant_be_removed_is_an_error_not_a_spin() {
        let dir = temp("stuck");
        // A name that leaves no room for the one it is moved aside to (over
        // 255 bytes): it can't be, by root too.
        let lock = dir.join(format!("{}.lock", "s".repeat(240)));
        std::fs::create_dir(&lock).unwrap();
        age(&lock, STORAGE_STALE + Duration::from_secs(1));
        let mut set = LockSet::new();
        let started = Instant::now();
        let taken = set.take(
            &lock,
            STORAGE_STALE,
            Instant::now() + Duration::from_millis(300),
        );
        match taken {
            Err(LockError::Io(why)) => assert!(why.contains("stale lock"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(lock.is_dir());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_stale_lock_is_moved_aside_before_it_goes() {
        let dir = temp("aside");
        let lock = dir.join(".oauth_refresh.lock");
        let put_back = |seen: &LockId, why: &str| {
            let before = identity(&lock).unwrap();
            remove_stale(&lock, seen).unwrap();
            assert_eq!(identity(&lock).as_ref(), Some(&before), "{why}: put back");
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "{why}");
        };
        std::fs::create_dir(&lock).unwrap();
        age(&lock, REFRESH_STALE + Duration::from_secs(1));
        let looked = identity(&lock).unwrap();
        // Between the look and the removal someone removed it as stale and
        // took a fresh lock, which the file system gave the same inode (ext4
        // often does; tmpfs doesn't, so it is set here on purpose).
        std::fs::remove_dir(&lock).unwrap();
        std::fs::create_dir(&lock).unwrap();
        let theirs = identity(&lock).unwrap();
        let reused = LockId {
            dev: theirs.dev,
            ino: theirs.ino,
            ..looked.clone()
        };
        put_back(&reused, "a new lock in the same inode");
        // Where the file system keeps no birth time, the modification time
        // alone tells them apart.
        let reused = LockId {
            birth_ns: None,
            ..reused
        };
        put_back(&reused, "the same, without birth times");
        // The holder touched it between the look and the removal: fresh
        // again, so not stale.
        age(&lock, REFRESH_STALE + Duration::from_secs(1));
        let looked = identity(&lock).unwrap();
        set_mtime(&lock, SystemTime::now()).unwrap();
        put_back(&looked, "a lock touched meanwhile");
        // The very one looked at goes (renaming it keeps its times).
        age(&lock, REFRESH_STALE + Duration::from_secs(1));
        let looked = identity(&lock).unwrap();
        remove_stale(&lock, &looked).unwrap();
        assert!(!lock.exists());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_directory_is_known_by_more_than_its_inode() {
        let then = LockId {
            path: "/h/.claude/.storage-write.lock".into(),
            dev: 1,
            ino: 7,
            mtime_ms: 1_000_000,
            birth_ns: Some(900_000_000_000),
        };
        assert!(same_dir(&then, &then.clone()));
        // Its path is not part of it: moved aside, it is still the one.
        assert!(same_dir(
            &then,
            &LockId {
                path: "/h/.claude/.storage-write.lock.stale-x".into(),
                ..then.clone()
            }
        ));
        for (other, why) in [
            (
                LockId {
                    ino: 8,
                    ..then.clone()
                },
                "another inode",
            ),
            (
                LockId {
                    dev: 2,
                    ..then.clone()
                },
                "another device",
            ),
            (
                LockId {
                    mtime_ms: 1_016_000,
                    birth_ns: Some(916_000_000_000),
                    ..then.clone()
                },
                "made again in the same inode",
            ),
            (
                LockId {
                    birth_ns: Some(900_000_000_001),
                    ..then.clone()
                },
                "born at another time",
            ),
            (
                LockId {
                    mtime_ms: 1_000_001,
                    ..then.clone()
                },
                "touched since",
            ),
            (
                LockId {
                    mtime_ms: 1_016_000,
                    birth_ns: None,
                    ..then.clone()
                },
                "no birth time, a new modification time",
            ),
        ] {
            assert!(!same_dir(&then, &other), "{why}");
        }
        // Without a birth time on either side, the rest decides.
        assert!(same_dir(
            &then,
            &LockId {
                birth_ns: None,
                ..then.clone()
            }
        ));
    }

    #[test]
    fn release_spares_a_lock_made_again_in_the_same_inode() {
        let dir = temp("release-reused");
        let lock = dir.join(".storage-write.lock");
        // The worker's lock went stale (the worker stopped, say), someone
        // removed it and took theirs, in the same inode.
        std::fs::create_dir(&lock).unwrap();
        let theirs = identity(&lock).unwrap();
        let held = LockId {
            mtime_ms: theirs.mtime_ms - (STORAGE_STALE.as_millis() as u64 + 1_000),
            birth_ns: theirs.birth_ns.map(|at| at - 16_000_000_000),
            ..theirs.clone()
        };
        drop(LockSet {
            held: vec![held.clone()],
        });
        assert_eq!(identity(&lock), Some(theirs.clone()), "theirs stays");
        drop(LockSet {
            held: vec![LockId {
                birth_ns: None,
                ..held
            }],
        });
        assert_eq!(identity(&lock), Some(theirs.clone()), "theirs stays");
        // Its own goes.
        drop(LockSet { held: vec![theirs] });
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
    fn the_worker_knows_its_locks_after_touching_them() {
        let dir = temp("reclaim-touched");
        let lock = dir.join("ours.lock");
        let mut set = LockSet::new();
        set.take(
            &lock,
            REFRESH_STALE,
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        // The journal noted it, then the swap touched it again, then the
        // worker crashed.
        let noted = set.ids();
        std::thread::sleep(Duration::from_millis(20));
        set.touch();
        assert_ne!(set.ids()[0].mtime_ms, noted[0].mtime_ms);
        std::mem::forget(set);
        reclaim(&noted);
        assert!(!lock.exists(), "reclaimed at once, not waited out");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_inode_used_again_is_not_the_worker_s_lock() {
        let noted = LockId {
            path: "/h/.claude/.oauth_refresh.lock".into(),
            dev: 1,
            ino: 7,
            mtime_ms: 1_000_000,
            birth_ns: None,
        };
        let touched = LockId {
            mtime_ms: 1_000_000 + 8_000,
            ..noted.clone()
        };
        assert!(is_own(&noted, &touched), "touched by the swap");
        // Made by someone in the same inode once the worker's had gone
        // stale: 15 s at the least after the worker's last touch, itself no
        // earlier than the note.
        let theirs = LockId {
            mtime_ms: 1_000_000 + 15_000,
            ..noted.clone()
        };
        assert!(!is_own(&noted, &theirs));
        assert!(!is_own(
            &noted,
            &LockId {
                mtime_ms: 1_000_000 + OWN_TOUCHES.as_millis() as u64,
                ..noted.clone()
            }
        ));
        assert!(!is_own(
            &noted,
            &LockId {
                mtime_ms: 1_000_000 - 1,
                ..noted.clone()
            }
        ));
        assert!(!is_own(
            &noted,
            &LockId {
                ino: 8,
                ..noted.clone()
            }
        ));
        // With birth times, those decide.
        let born = LockId {
            birth_ns: Some(5),
            ..noted.clone()
        };
        assert!(is_own(
            &born,
            &LockId {
                mtime_ms: 9_999_999,
                ..born.clone()
            }
        ));
        assert!(!is_own(
            &born,
            &LockId {
                birth_ns: Some(6),
                ..born.clone()
            }
        ));
    }

    #[test]
    fn a_lock_put_back_never_replaces_a_new_one() {
        use std::os::unix::fs::MetadataExt;
        let dir = temp("put-back");
        let (aside, lock) = (dir.join("x.lock.stale-1"), dir.join("x.lock"));
        std::fs::create_dir(&aside).unwrap();
        std::fs::create_dir(&lock).unwrap();
        let theirs = std::fs::metadata(&lock).unwrap().ino();
        let refused = rename_no_replace(&aside, &lock).unwrap_err();
        assert_eq!(refused.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::metadata(&lock).unwrap().ino(), theirs);
        assert!(aside.is_dir());
        std::fs::remove_dir(&lock).unwrap();
        rename_no_replace(&aside, &lock).unwrap();
        assert!(lock.is_dir() && !aside.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn what_was_moved_aside_goes_once_stale() {
        let dir = temp("sweep");
        let lock = dir.join(".storage-write.lock");
        let aside = |tag: &str| dir.join(format!(".storage-write.lock.stale-{tag}"));
        let old = aside(&"a".repeat(32));
        let fresh = aside(&"b".repeat(32));
        let other = aside("not-ours");
        let full = aside(&"c".repeat(32));
        for path in [&old, &fresh, &other, &full] {
            std::fs::create_dir(path).unwrap();
        }
        std::fs::write(full.join("x"), "").unwrap();
        for path in [&old, &other, &full] {
            age(path, STORAGE_STALE * 2);
        }
        // Taking the lock sweeps first.
        let mut set = LockSet::new();
        set.take(
            &lock,
            STORAGE_STALE,
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        assert!(!old.exists(), "ours, stale: gone");
        assert!(fresh.is_dir(), "not stale yet");
        assert!(other.is_dir(), "not a name the worker gives");
        assert!(full.is_dir(), "not empty: not a lock");
        drop(set);
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
