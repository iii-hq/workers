#[cfg(unix)]
use std::time::Duration;
use std::{fs, path::Path, sync::Mutex};

use rand::RngCore;
use rusqlite::{Connection, OpenFlags, TransactionBehavior};

#[cfg(test)]
mod persistence_tests;
pub mod rows;
#[cfg(test)]
mod runtime_tests;
mod sql;

struct Database {
    connection: Connection,
    data: Data,
    next_prune: i64,
}

/// Synchronous compatibility for lifecycle callers on the production multi-thread
/// runtime. Tokio hands off the executor core before disk I/O or mutex waits.
/// Current-thread callers must use wiring::storage_task/spawn_blocking; plain
/// synchronous callers and unit tests can call directly without a Tokio runtime.
fn blocking<T>(f: impl FnOnce() -> T) -> T {
    if tokio::runtime::Handle::try_current()
        .is_ok_and(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread)
    {
        tokio::task::block_in_place(f)
    } else {
        f()
    }
}

use super::{types::Data, Failure, Result};

/// SQLite is a transactional inbox/outbox, not an execution queue. Execution is
/// delegated to the installed queue worker. FULL sync precedes every HTTP ack.
pub struct Store {
    database: Mutex<Database>,
    _lock: fs::File,
}
impl Store {
    #[cfg(not(unix))]
    pub fn open(_path: &Path) -> Result<Self> {
        Err(Failure::Invalid(
            "webhook persistence currently requires Unix file ownership and exclusive locking"
                .into(),
        ))
    }
    #[cfg(unix)]
    pub fn open(path: &Path) -> Result<Self> {
        blocking(|| Self::open_blocking(path))
    }
    #[cfg(unix)]
    fn open_blocking(path: &Path) -> Result<Self> {
        let parent = path
            .parent()
            .ok_or_else(|| Failure::Invalid("storage_path needs a parent".into()))?;
        fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
            for p in [parent, path] {
                if p.exists() {
                    let m = fs::symlink_metadata(p)?;
                    if m.file_type().is_symlink() || m.uid() != unsafe { libc::geteuid() } {
                        return Err(Failure::Invalid(
                            "storage must be owned by worker, without symlinks".into(),
                        ));
                    }
                }
            }
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        let lock_path = path.with_extension("lock");
        let mut options = fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let lock = options.open(lock_path)?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: flock receives a live file descriptor; the File outlives Store.
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(Failure::Invalid(
                    "another process owns this webhook installation".into(),
                ));
            }
        }
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(2))?;
        conn.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;")?;
        let data = sql::initialize(&mut conn)?;
        Ok(Self {
            database: Mutex::new(Database {
                connection: conn,
                data,
                next_prune: 0,
            }),
            _lock: lock,
        })
    }
    /// Cheap immutable snapshot. Payloads are shared, not cloned/deserialized.
    pub fn read(&self) -> Result<Data> {
        blocking(|| {
            Ok(self
                .database
                .lock()
                .map_err(|_| Failure::StoragePoisoned)?
                .data
                .clone())
        })
    }
    /// Read-only, transaction-consistent inspection for integration tests/tools.
    /// Does not acquire installation ownership, migrate, or write to the database.
    /// Contains secrets: never expose this snapshot as a public function response.
    pub fn inspect(path: &Path) -> Result<Data> {
        blocking(|| {
            let mut conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            conn.busy_timeout(std::time::Duration::from_secs(2))?;
            let tx = conn.transaction()?;
            let data = sql::load(&tx)?;
            tx.commit()?;
            Ok(data)
        })
    }
    pub fn change<T>(&self, f: impl FnOnce(&mut Data) -> Result<T>) -> Result<T> {
        blocking(|| {
            let mut db = self.database.lock().map_err(|_| Failure::StoragePoisoned)?;
            // Copy-on-write row indexes provide rollback without cloning payloads.
            // Cache replacement only follows successful FULL-sync commit.
            let mut data = db.data.clone();
            let out = f(&mut data)?;
            let now = chrono::Utc::now().timestamp();
            let prune = now >= db.next_prune;
            let Database {
                connection,
                data: old,
                ..
            } = &mut *db;
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            sql::persist(&tx, old, &mut data, now)?;
            if prune {
                sql::prune(&tx, &mut data, now)?;
            }
            tx.commit()?;
            sql::clean(&mut data);
            db.data = data;
            if prune {
                db.next_prune = now.saturating_add(60);
            }
            Ok(out)
        })
    }
}
pub fn random_id() -> String {
    let mut bytes = [0; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
