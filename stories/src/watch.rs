//! One recursive watcher per workspace root. Events are debounced; a flush
//! asks for a working-tree rebuild when a story file or any input of the
//! current index changed. Build folders, node_modules, the data folder and
//! `.git` never count.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

use crate::builder::Ctx;
use crate::store::WORKTREE_KEY;

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
/// is an input), and the worker's own data folder never count.
pub fn is_ignored(path: &Path, data_dir: &Path) -> bool {
    if path.starts_with(data_dir) {
        return true;
    }
    path.components().any(|c| {
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
    for workspace in config.workspaces {
        let root = workspace.path_resolved();
        if !root.is_dir() {
            tracing::warn!(workspace = workspace.name, path = %root.display(), "workspace path missing; not watching");
            continue;
        }
        let name = workspace.name.clone();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name(format!("stories-watch-{name}"))
            .spawn(move || watch_blocking(ctx, name, root))
            .ok();
    }
}

fn watch_blocking(ctx: Arc<Ctx>, name: String, root: PathBuf) {
    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = match notify::recommended_watcher(tx) {
        Ok(w) => w,
        Err(error) => {
            tracing::warn!(workspace = name, error = %error, "cannot create watcher");
            return;
        }
    };
    if let Err(error) = watcher.watch(&root, RecursiveMode::Recursive) {
        tracing::warn!(workspace = name, error = %error, "cannot watch workspace");
        return;
    }
    tracing::info!(workspace = name, path = %root.display(), "watching for story changes");
    let data_dir = ctx.config().data_path_resolved();
    while let Ok(first) = rx.recv() {
        let started = std::time::Instant::now();
        let mut changed: Vec<PathBuf> = Vec::new();
        let mut collect = |event: notify::Result<notify::Event>| {
            if let Ok(event) = event {
                for path in event.paths {
                    if !is_ignored(&path, &data_dir) && !changed.contains(&path) {
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
        let inputs: HashSet<String> = ctx
            .store
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
            let _ = ctx.rebuild_tx.send(name.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_build_folders_and_the_data_dir() {
        let data = Path::new("/ws/data/stories");
        assert!(is_ignored(
            Path::new("/ws/app/node_modules/react/index.js"),
            data
        ));
        assert!(is_ignored(Path::new("/ws/data/stories/files/abc"), data));
        assert!(is_ignored(Path::new("/ws/.git/index"), data));
        assert!(is_ignored(
            Path::new("/ws/.iii/compose/default/logs/stories.log"),
            data
        ));
        assert!(is_ignored(Path::new("/ws/data/observability/x"), data));
        assert!(!is_ignored(
            Path::new("/ws/app/.storybook/preview.tsx"),
            data
        ));
        assert!(!is_ignored(Path::new("/ws/app/src/Button.tsx"), data));
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
