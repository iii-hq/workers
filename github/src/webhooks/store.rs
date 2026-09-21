#[cfg(unix)]
use std::time::Duration;
use std::{fs, path::Path, sync::Mutex};

use rand::RngCore;
use rusqlite::{params, Connection, TransactionBehavior};

use super::{types::Data, Failure, Result};

/// SQLite is a transactional inbox/outbox, not an execution queue. Execution is
/// delegated to the installed queue worker. FULL sync precedes every HTTP ack.
pub struct Store {
    connection: Mutex<Connection>,
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
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(2))?;
        conn.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS state (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL);")?;
        let data = Data {
            installation: random_id(),
            ..Default::default()
        };
        conn.execute(
            "INSERT OR IGNORE INTO state VALUES(1, ?1)",
            [serde_json::to_string(&data)?],
        )?;
        Ok(Self {
            connection: Mutex::new(conn),
            _lock: lock,
        })
    }
    pub fn read(&self) -> Result<Data> {
        let conn = self
            .connection
            .lock()
            .map_err(|_| Failure::StoragePoisoned)?;
        let text: String =
            conn.query_row("SELECT value FROM state WHERE id=1", [], |r| r.get(0))?;
        Ok(serde_json::from_str(&text)?)
    }
    pub fn change<T>(&self, f: impl FnOnce(&mut Data) -> Result<T>) -> Result<T> {
        let mut conn = self
            .connection
            .lock()
            .map_err(|_| Failure::StoragePoisoned)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let text: String = tx.query_row("SELECT value FROM state WHERE id=1", [], |r| r.get(0))?;
        let mut data: Data = serde_json::from_str(&text)?;
        let out = f(&mut data)?;
        tx.execute(
            "UPDATE state SET value=?1 WHERE id=1",
            params![serde_json::to_string(&data)?],
        )?;
        tx.commit()?;
        Ok(out)
    }
}
pub fn random_id() -> String {
    let mut bytes = [0; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
