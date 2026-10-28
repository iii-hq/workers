//! One recursive watcher per workspace root. Events are debounced; a flush
//! asks for a working-tree rebuild when a story file or any input of the
//! current index changed. Build folders, node_modules, the data folder and
//! `.git` never count.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use tokio::sync::mpsc::UnboundedSender;

use crate::builder::Ctx;
use crate::store::{Store, WORKTREE_KEY};

const DEBOUNCE: Duration = Duration::from_millis(500);
/// A burst of events is flushed after this long even if it never goes
/// quiet: log files under the workspace churn forever, and a flush that
/// waits for silence would never happen.
const MAX_BURST: Duration = Duration::from_secs(3);
const SKIP_DIRS: [&str; 10] = [
    "node_modules",
    "dist",
    "build",
    "target",
    "coverage",
    "out",
    "data",
    "storybook-static",
    "tmp",
    "logs",
];

/// Build folders, every hidden folder except `.storybook` (the preview file
/// is an input), and the worker's own data folder never count. Folders are
/// judged below `root`, so a workspace under `/tmp` or `~/.x` still counts.
pub fn is_ignored(path: &Path, root: &Path, data_dir: &Path) -> bool {
    if path.starts_with(data_dir) {
        return true;
    }
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components().any(|c| {
        let name = c.as_os_str().to_string_lossy();
        SKIP_DIRS.contains(&name.as_ref())
            || (name.starts_with('.') && name != "." && name != ".storybook")
    })
}

fn is_story_file(path: &Path) -> bool {
    path.file_name()
        .map(|n| n.to_string_lossy().contains(".stories."))
        .unwrap_or(false)
}

/// Whether a flush of changed paths should rebuild: any story file, or any
/// path the current index lists as an input.
pub fn relevant(changed: &[PathBuf], root: &Path, inputs: &HashSet<String>) -> bool {
    changed.iter().any(|path| {
        if is_story_file(path) {
            return true;
        }
        let rel = path
            .strip_prefix(root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"));
        inputs.contains(&rel)
    })
}

pub fn spawn_all(ctx: Arc<Ctx>) {
    let config = ctx.config();
    if !config.watch {
        tracing::info!("stories watcher disabled by configuration");
        return;
    }
    let data_dir = config.data_path_resolved();
    for workspace in config.workspaces {
        let root = workspace.path_resolved();
        if !root.is_dir() {
            tracing::warn!(workspace = workspace.name, path = %root.display(), "workspace path missing; not watching");
            continue;
        }
        let name = workspace.name.clone();
        let ctx = ctx.clone();
        let data_dir = data_dir.clone();
        std::thread::Builder::new()
            .name(format!("stories-watch-{name}"))
            .spawn(move || watch_blocking(&ctx.store, &ctx.rebuild_tx, name, root, &data_dir))
            .ok();
    }
}

fn watch_blocking(
    store: &Store,
    rebuild_tx: &UnboundedSender<String>,
    name: String,
    root: PathBuf,
    data_dir: &Path,
) {
    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = match notify::recommended_watcher(tx) {
        Ok(w) => w,
        Err(error) => {
            tracing::warn!(workspace = name, error = %error, "cannot create watcher");
            return;
        }
    };
    // FSEvents reports resolved paths (`/var/..` arrives as `/private/var/..`),
    // so events are matched against resolved paths; watching the resolved
    // root makes inotify report that same form.
    let root = root.canonicalize().unwrap_or(root);
    let data_dir = data_dir
        .canonicalize()
        .unwrap_or_else(|_| data_dir.to_path_buf());
    if let Err(error) = watcher.watch(&root, RecursiveMode::Recursive) {
        tracing::warn!(workspace = name, error = %error, "cannot watch workspace");
        return;
    }
    tracing::info!(workspace = name, path = %root.display(), "watching for story changes");
    while let Ok(first) = rx.recv() {
        // The build loop is gone, so nothing would rebuild: stop watching.
        if rebuild_tx.is_closed() {
            return;
        }
        let started = std::time::Instant::now();
        let mut changed: Vec<PathBuf> = Vec::new();
        let mut collect = |event: notify::Result<notify::Event>| {
            if let Ok(event) = event {
                for path in event.paths {
                    if !is_ignored(&path, &root, &data_dir) && !changed.contains(&path) {
                        changed.push(path);
                    }
                }
            }
        };
        collect(first);
        while started.elapsed() < MAX_BURST {
            match rx.recv_timeout(DEBOUNCE) {
                Ok(event) => collect(event),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
        if changed.is_empty() {
            continue;
        }
        let inputs: HashSet<String> = store
            .read_index(&name, WORKTREE_KEY)
            .map(|index| {
                index
                    .components
                    .into_iter()
                    .flat_map(|c| c.inputs.into_iter().map(|i| i.path))
                    .collect()
            })
            .unwrap_or_default();
        if relevant(&changed, &root, &inputs) {
            tracing::debug!(
                workspace = name,
                files = changed.len(),
                "story inputs changed; rebuilding"
            );
            let _ = rebuild_tx.send(name.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_build_folders_and_the_data_dir() {
        let root = Path::new("/ws");
        let data = Path::new("/ws/data/stories");
        assert!(is_ignored(
            Path::new("/ws/app/node_modules/react/index.js"),
            root,
            data
        ));
        assert!(is_ignored(
            Path::new("/ws/data/stories/files/abc"),
            root,
            data
        ));
        assert!(is_ignored(Path::new("/ws/.git/index"), root, data));
        assert!(is_ignored(
            Path::new("/ws/.iii/compose/default/logs/stories.log"),
            root,
            data
        ));
        assert!(is_ignored(
            Path::new("/ws/data/observability/x"),
            root,
            data
        ));
        assert!(!is_ignored(
            Path::new("/ws/app/.storybook/preview.tsx"),
            root,
            data
        ));
        assert!(!is_ignored(Path::new("/ws/app/src/Button.tsx"), root, data));
        assert!(!is_ignored(
            Path::new("/tmp/.x/ws/app/src/Button.tsx"),
            Path::new("/tmp/.x/ws"),
            data
        ));
    }

    /// A workspace reached through a symlink (macOS `/var` tempdirs, a linked
    /// checkout): FSEvents reports resolved paths, inotify the watched form,
    /// and the data folder is configured in yet another form.
    #[cfg(unix)]
    #[test]
    fn symlinked_workspace_rebuilds_on_an_input_and_ignores_the_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        std::fs::create_dir_all(real.join("src")).unwrap();
        std::fs::write(real.join("src/Button.tsx"), "export const v = 0;\n").unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let data_dir = real.join("store");
        let store = Store::new(data_dir.clone());
        let index = store.line_dir("ws", WORKTREE_KEY).join("index.json");
        std::fs::create_dir_all(index.parent().unwrap()).unwrap();
        let input = serde_json::json!({ "path": "src/Button.tsx", "hop": 1, "sha256": "x" });
        let index_json = serde_json::json!({
            "workspace": "ws",
            "line": { "key": WORKTREE_KEY, "kind": "worktree", "label": "working tree", "built_at": "t" },
            "components": [{
                "id": "button", "project": ".", "file": "src/Button.stories.tsx",
                "path": "src/Button.stories.tsx", "title": "Button", "group": "",
                "version": "v", "inputs": [input],
            }],
        });
        std::fs::write(&index, index_json.to_string()).unwrap();
        assert!(store.read_index("ws", WORKTREE_KEY).is_some());

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let watcher = {
            let (link, data_dir) = (link.clone(), data_dir.clone());
            std::thread::spawn(move || watch_blocking(&store, &tx, "ws".into(), link, &data_dir))
        };
        let mut rebuilt = |within: Duration| {
            let deadline = std::time::Instant::now() + within;
            while std::time::Instant::now() < deadline {
                if rx.try_recv().is_ok() {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            false
        };

        // Rewrite until the watcher thread is up and reports the edit.
        let live = (1..=5).any(|i| {
            std::fs::write(
                link.join("src/Button.tsx"),
                format!("export const v = {i};\n"),
            )
            .unwrap();
            rebuilt(Duration::from_secs(2))
        });
        assert!(live, "an input edited through the link must rebuild");
        while rebuilt(Duration::from_secs(1)) {}

        std::fs::write(data_dir.join("Copy.stories.tsx"), "export default {};\n").unwrap();
        assert!(
            !rebuilt(Duration::from_millis(1500)),
            "a story file in the data folder must not rebuild"
        );

        drop(rx);
        let stopped = (6..=10).any(|i| {
            std::fs::write(
                link.join("src/Button.tsx"),
                format!("export const v = {i};\n"),
            )
            .unwrap();
            std::thread::sleep(Duration::from_secs(1));
            watcher.is_finished()
        });
        assert!(stopped, "the watcher must stop once nobody takes rebuilds");
        watcher.join().unwrap();
    }

    #[test]
    fn relevance_needs_a_story_file_or_a_known_input() {
        let root = Path::new("/ws");
        let inputs: HashSet<String> = ["app/src/Button.tsx".to_string()].into_iter().collect();
        assert!(relevant(
            &[PathBuf::from("/ws/app/src/Button.tsx")],
            root,
            &inputs
        ));
        assert!(relevant(
            &[PathBuf::from("/ws/app/src/New.stories.tsx")],
            root,
            &inputs
        ));
        assert!(!relevant(
            &[PathBuf::from("/ws/app/src/Other.tsx")],
            root,
            &inputs
        ));
    }
}
