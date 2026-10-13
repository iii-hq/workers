//! Following edits made to `.env` by hand, without polling: the operating
//! system reports changes in the project directory (inotify, FSEvents or
//! ReadDirectoryChangesW, through `notify`'s recommended watcher — never its
//! polling fallback), and each burst of events naming `.env` re-reads the
//! shared variables once. `secrets::changed` then carries what changed.
//!
//! The directory is watched rather than the file: an editor that saves by
//! writing a new file and renaming it over `.env` replaces the inode a
//! file watch would follow. The watch is not recursive, so writes under
//! `data/` or `config/` never wake it.
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::functions::Ctx;

/// A save arrives as several events (truncate and write, or create and
/// rename); they are taken together once the directory has been quiet this
/// long.
pub const SETTLE: Duration = Duration::from_millis(150);

/// Keeps the watch alive; dropping it stops following `.env`.
pub struct EnvWatch {
    _watcher: RecommendedWatcher,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for EnvWatch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The directories to watch and the names in them that are `.env`: the
/// path itself and, when it is a symlink, the file it points at.
fn targets(dotenv: &Path) -> Vec<(PathBuf, OsString)> {
    let mut out = Vec::new();
    // FSEvents reports canonical paths; a `.env` not created yet is
    // canonicalized through its directory.
    let canonical = std::fs::canonicalize(dotenv).ok().or_else(|| {
        let dir = std::fs::canonicalize(dotenv.parent()?).ok()?;
        Some(dir.join(dotenv.file_name()?))
    });
    for path in [Some(dotenv.to_path_buf()), canonical]
        .into_iter()
        .flatten()
    {
        if let (Some(dir), Some(name)) = (path.parent(), path.file_name()) {
            let target = (dir.to_path_buf(), name.to_os_string());
            if !out.contains(&target) {
                out.push(target);
            }
        }
    }
    out
}

/// Whether `event` is about one of `targets`' files. Reads are not changes.
pub fn touches(event: &Event, targets: &[(PathBuf, OsString)]) -> bool {
    !matches!(event.kind, EventKind::Access(_))
        && event.paths.iter().any(|path| {
            targets.iter().any(|(dir, name)| {
                path.file_name() == Some(name.as_os_str()) && path.parent() == Some(dir.as_path())
            })
        })
}

/// Follow the env file the store reads now, replacing any earlier watch.
/// Returns whether a watch is running.
pub async fn follow(ctx: &Arc<Ctx>) -> bool {
    let watch = match ctx.store.dotenv_path().await {
        Some(dotenv) => spawn(Arc::downgrade(ctx), &dotenv),
        None => None,
    };
    let running = watch.is_some();
    *ctx.env_watch.lock().unwrap_or_else(|p| p.into_inner()) = watch;
    running
}

/// Start following `dotenv`. `None` when the platform watcher cannot start
/// (no directory, out of inotify watches): edits made by hand then reach a
/// consumer on its next resolution, and are not announced.
fn spawn(ctx: Weak<Ctx>, dotenv: &Path) -> Option<EnvWatch> {
    let targets = targets(dotenv);
    let (tx, rx) = mpsc::unbounded_channel::<()>();
    let filter = targets.clone();
    let mut watcher = match notify::recommended_watcher(move |event: notify::Result<Event>| {
        if event.is_ok_and(|event| touches(&event, &filter)) {
            let _ = tx.send(());
        }
    }) {
        Ok(watcher) => watcher,
        Err(error) => {
            tracing::warn!(error = %error, "cannot follow .env edits; they apply on the next resolution");
            return None;
        }
    };
    for (dir, _) in &targets {
        if let Err(error) = watcher.watch(dir, RecursiveMode::NonRecursive) {
            tracing::warn!(dir = %dir.display(), error = %error, "cannot follow .env edits; they apply on the next resolution");
            return None;
        }
    }
    tracing::info!(path = %dotenv.display(), "following .env edits");
    Some(EnvWatch {
        _watcher: watcher,
        task: tokio::spawn(run(ctx, rx)),
    })
}

/// Holds the context weakly: the context owns this watch.
async fn run(ctx: Weak<Ctx>, mut rx: mpsc::UnboundedReceiver<()>) {
    while rx.recv().await.is_some() {
        // Take the rest of the burst before reading the file.
        loop {
            match tokio::time::timeout(SETTLE, rx.recv()).await {
                Ok(Some(())) => continue,
                Ok(None) => return,
                Err(_) => break,
            }
        }
        let Some(ctx) = ctx.upgrade() else { return };
        for event in ctx.store.env_changes().await {
            ctx.changed(&event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, CreateKind, ModifyKind, RenameMode};

    fn event(kind: EventKind, path: &str) -> Event {
        Event::new(kind).add_path(PathBuf::from(path))
    }

    #[test]
    fn only_changes_to_the_env_file_count() {
        let targets = vec![(PathBuf::from("/p"), OsString::from(".env"))];
        let modify = EventKind::Modify(ModifyKind::Any);
        assert!(touches(&event(modify, "/p/.env"), &targets));
        assert!(touches(
            &event(
                EventKind::Modify(ModifyKind::Name(RenameMode::To)),
                "/p/.env"
            ),
            &targets
        ));
        assert!(touches(
            &event(EventKind::Create(CreateKind::File), "/p/.env"),
            &targets
        ));
        assert!(!touches(&event(modify, "/p/.env.example"), &targets));
        assert!(!touches(&event(modify, "/p/data/.env"), &targets));
        assert!(!touches(&event(modify, "/p/worker-compose.yaml"), &targets));
        assert!(!touches(
            &event(EventKind::Access(AccessKind::Any), "/p/.env"),
            &targets
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_env_is_followed_where_it_points() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("shared/keys.env");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, "A=1\n").unwrap();
        let project = root.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::os::unix::fs::symlink(&real, project.join(".env")).unwrap();
        let found = targets(&project.join(".env"));
        assert_eq!(found[0], (project.clone(), OsString::from(".env")));
        assert_eq!(found[1].1, OsString::from("keys.env"));
    }
}
