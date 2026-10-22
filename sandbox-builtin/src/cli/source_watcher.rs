// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Legacy restart watcher and shared host-watch helpers.
//!
//! Virtiofs propagates file content and metadata from the host into the
//! guest VM, but inotify events do NOT cross the FUSE boundary — so
//! watchers inside the VM (tsx watch, node --watch, cargo watch, etc.)
//! never fire on host edits unless they opt into polling. Some runtimes
//! (notably tsx 4.x) don't support a polling fallback at all.
//!
//! New VM launches use `source_sync_host`: it reuses the path filters and
//! watch-tree helpers in this module, then forwards file operations into the
//! guest.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use notify::{RecursiveMode, Watcher};

/// Debounce window: collapse rapid writes (editor save-then-rename,
/// multi-file renames, etc.) into one operation.
pub const DEBOUNCE_MS: u64 = 500;

/// Directories whose contents should NEVER trigger a restart. These are
/// the high-churn artifact directories that tooling writes to during
/// normal operation — including during the restart itself, which would
/// otherwise produce an infinite restart loop.
pub const IGNORED_DIR_NAMES: &[&str] = &[
    // Node / TypeScript
    "node_modules",
    ".pnpm-store",
    ".parcel-cache",
    ".turbo",
    ".next",
    ".nuxt",
    ".svelte-kit",
    // Rust
    "target",
    // Python
    ".venv",
    "venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    // Go / PHP / Ruby share "vendor"
    "vendor",
    // Ruby
    ".bundle",
    // JVM (Maven, Gradle)
    ".gradle",
    ".m2",
    // .NET
    "obj",
    ".nuget",
    // Elixir / Erlang
    "_build",
    "deps",
    // Generic build output
    "dist",
    "build",
    "out",
    "coverage",
    ".cache",
    // VCS
    ".git",
    ".svn",
    ".hg",
    // IDE / OS
    ".idea",
    ".vscode",
    ".DS_Store",
];

/// O(1) lookup set for ignore-path classification. Built once; hit on
/// every notify event, which can fire thousands of times per second
/// during bulk operations.
static IGNORED_DIR_SET: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| IGNORED_DIR_NAMES.iter().copied().collect());

/// Returns true if a notify event on this path should be ignored.
///
/// A path is ignored when any of its components (after stripping the
/// project root prefix) matches [`IGNORED_DIR_NAMES`]. This correctly
/// ignores writes deep inside `node_modules/.cache/...` as well as the
/// directory itself.
///
/// Hidden files at the project root (e.g. `.env.local`) are NOT
/// ignored — users frequently edit those and expect a restart. Only
/// named artifact dirs are filtered.
pub fn should_ignore_path(path: &Path, project_root: &Path) -> bool {
    let rel = path.strip_prefix(project_root).unwrap_or(path);
    for component in rel.components() {
        if let std::path::Component::Normal(os) = component
            && let Some(s) = os.to_str()
            && IGNORED_DIR_SET.contains(s)
        {
            return true;
        }
    }
    false
}

/// Register non-recursive watches on every directory under `root`
/// EXCEPT those whose name (or any ancestor's name) matches
/// [`IGNORED_DIR_NAMES`]. This prunes inotify watch registration at
/// setup time, avoiding thousands of useless descriptors inside
/// `node_modules`/`target`/`.git` and cutting per-event filter cost
/// (events from ignored subtrees never fire at all).
///
/// Best-effort: permission errors on subdirs are logged and skipped,
/// not propagated. The watcher stays online for the rest of the tree.
pub(crate) fn watch_pruned(
    watcher: &mut notify::RecommendedWatcher,
    root: &Path,
    registered: &mut HashSet<PathBuf>,
) -> anyhow::Result<()> {
    use std::collections::VecDeque;

    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    queue.push_back(root.to_path_buf());

    while let Some(dir) = queue.pop_front() {
        match watcher.watch(&dir, RecursiveMode::NonRecursive) {
            Ok(()) => {
                registered.insert(dir.clone());
            }
            Err(e) => {
                // Detect inotify ENOSPC (io::ErrorKind::StorageFull on
                // stable) and call it out once with an actionable
                // remediation. notify wraps the OS error in its own
                // variants so we match on the error string rather than
                // plumbing through a platform-specific check.
                //
                // ENOSPC is terminal: the kernel's inotify budget is
                // a per-user counter, so every remaining `inotify_add_watch`
                // for this user will fail the same way. Continuing the
                // walk would burn one syscall per dir on large trees
                // (tens of thousands on a monorepo). Abort the walk —
                // the watcher stays online for already-registered dirs,
                // and the user now has an actionable log line.
                let msg = e.to_string();
                if msg.contains("No space left on device")
                    || msg.contains("watch limit")
                    || msg.contains("ENOSPC")
                {
                    tracing::warn!(
                        path = %dir.display(),
                        error = %e,
                        "source watcher: inotify watch budget exhausted — file changes in \
                         some directories will go undetected. Raise the limit with: \
                         `sudo sysctl fs.inotify.max_user_watches=524288` (persist via \
                         /etc/sysctl.d/). Aborting the rest of the walk to avoid a storm \
                         of failing syscalls."
                    );
                    return Ok(());
                }
                tracing::debug!(
                    path = %dir.display(),
                    error = %e,
                    "watch_pruned: skipping unwatchable dir"
                );
                continue;
            }
        }

        let read = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(
                    path = %dir.display(),
                    error = %e,
                    "watch_pruned: cannot enumerate, skipping children"
                );
                continue;
            }
        };

        for entry in read.flatten() {
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if !ft.is_dir() || ft.is_symlink() {
                continue;
            }
            let name = entry.file_name();
            if let Some(s) = name.to_str()
                && IGNORED_DIR_SET.contains(s)
            {
                continue;
            }
            queue.push_back(entry.path());
        }
    }

    Ok(())
}

/// Register `NonRecursive` watches on any paths from a burst that turn
/// out to be non-ignored directories. The initial `watch_pruned` pass
/// only covers directories that exist at startup; this catches
/// post-startup `mkdir` calls so edits inside new subdirs don't go
/// silent on Linux. Paths that aren't directories (files, stat errors,
/// ignored-subtree matches) are skipped. Duplicate registrations are
/// harmless — `notify` treats `watch(existing_path)` as a no-op on
/// inotify. Errors are logged at debug level and swallowed so a single
/// unwatchable dir doesn't tear down the watcher.
///
/// macOS FSEvents is inherently recursive and auto-covers new subdirs
/// already, so this call is a no-op there aside from the logging cost.
pub(crate) fn register_new_dirs(
    watcher: &mut notify::RecommendedWatcher,
    paths: &[PathBuf],
    root: &Path,
    registered: &mut HashSet<PathBuf>,
) {
    // `registered` persists across bursts: it's seeded from `watch_pruned`
    // and grown each time we successfully register a new dir. Skipping
    // already-registered paths collapses per-burst work to O(new unique
    // dirs) instead of O(events) — crucial under IDE-save-heavy loops
    // where the same parent directories appear in every burst.
    for p in paths {
        if registered.contains(p) {
            continue;
        }
        // Cheap rejection first — ignored subtrees (node_modules etc.)
        // must never be watched even if they were just created.
        if should_ignore_path(p, root) {
            continue;
        }
        // Use `symlink_metadata` so a user-placed symlink-to-dir in
        // the project doesn't silently register a watch outside the
        // project root. `watch_pruned` already skips symlinks on the
        // initial walk; match its policy here. Errors from stat
        // (file already gone, permission denied, race with delete)
        // are treated as "not a watchable dir".
        match std::fs::symlink_metadata(p) {
            Ok(md) if md.file_type().is_dir() => {}
            _ => continue,
        }
        // Defense-in-depth bounds check: reject any path that isn't
        // under the canonicalized project root before calling
        // `watcher.watch()`. The existing defenses (ignore-filter +
        // symlink rejection) already close today's attack paths, but
        // if a future notify refactor ever surfaces an out-of-root
        // event path (e.g. FSEvents resolving a mount alias), the
        // filter wouldn't catch it. `starts_with` is O(segments) and
        // uses canonical equality since `root` is already canonical
        // (`root_for_filter`); no additional syscall cost.
        if !p.starts_with(root) {
            continue;
        }
        match watcher.watch(p, RecursiveMode::NonRecursive) {
            Ok(()) => {
                registered.insert(p.clone());
            }
            Err(e) => {
                tracing::debug!(
                    path = %p.display(),
                    error = %e,
                    "source watcher: could not register new dir, skipping"
                );
            }
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/proj")
    }

    #[test]
    fn ignores_node_modules_root() {
        assert!(should_ignore_path(
            &PathBuf::from("/proj/node_modules"),
            &root()
        ));
    }

    #[test]
    fn ignores_nested_under_node_modules() {
        assert!(should_ignore_path(
            &PathBuf::from("/proj/node_modules/foo/bar.js"),
            &root()
        ));
    }

    #[test]
    fn ignores_target() {
        assert!(should_ignore_path(
            &PathBuf::from("/proj/target/debug/build.rs"),
            &root()
        ));
    }

    #[test]
    fn ignores_git_dir() {
        assert!(should_ignore_path(
            &PathBuf::from("/proj/.git/HEAD"),
            &root()
        ));
    }

    #[test]
    fn ignores_python_artifact_dirs() {
        assert!(should_ignore_path(
            &PathBuf::from("/proj/.venv/bin/python"),
            &root()
        ));
        assert!(should_ignore_path(
            &PathBuf::from("/proj/src/__pycache__/mod.cpython.pyc"),
            &root()
        ));
    }

    #[test]
    fn ignores_next_build_dirs() {
        assert!(should_ignore_path(
            &PathBuf::from("/proj/.next/cache/foo"),
            &root()
        ));
        assert!(should_ignore_path(
            &PathBuf::from("/proj/dist/index.js"),
            &root()
        ));
    }

    #[test]
    fn ignores_go_vendor() {
        assert!(should_ignore_path(
            &PathBuf::from("/proj/vendor/github.com/pkg/errors/errors.go"),
            &root()
        ));
    }

    #[test]
    fn ignores_jvm_build_dirs() {
        assert!(should_ignore_path(
            &PathBuf::from("/proj/.gradle/caches/x"),
            &root()
        ));
        assert!(should_ignore_path(
            &PathBuf::from("/proj/.m2/repository/a/b.jar"),
            &root()
        ));
    }

    #[test]
    fn ignores_dotnet_artifacts() {
        assert!(should_ignore_path(
            &PathBuf::from("/proj/obj/Debug/Foo.dll"),
            &root()
        ));
        assert!(should_ignore_path(
            &PathBuf::from("/proj/.nuget/packages/x"),
            &root()
        ));
    }

    #[test]
    fn ignores_elixir_artifacts() {
        assert!(should_ignore_path(
            &PathBuf::from("/proj/_build/dev/lib/app.beam"),
            &root()
        ));
        assert!(should_ignore_path(
            &PathBuf::from("/proj/deps/phoenix/mix.exs"),
            &root()
        ));
    }

    #[test]
    fn ignores_ruby_bundle() {
        assert!(should_ignore_path(
            &PathBuf::from("/proj/.bundle/config"),
            &root()
        ));
    }

    #[test]
    fn ignores_python_extended_caches() {
        assert!(should_ignore_path(
            &PathBuf::from("/proj/.mypy_cache/3.11/foo.data.json"),
            &root()
        ));
        assert!(should_ignore_path(
            &PathBuf::from("/proj/.ruff_cache/0.1.0/foo.py"),
            &root()
        ));
        assert!(should_ignore_path(
            &PathBuf::from("/proj/.tox/py311/lib/x"),
            &root()
        ));
    }

    #[test]
    fn ignores_pnpm_and_parcel_caches() {
        assert!(should_ignore_path(
            &PathBuf::from("/proj/.pnpm-store/v3/foo"),
            &root()
        ));
        assert!(should_ignore_path(
            &PathBuf::from("/proj/.parcel-cache/foo"),
            &root()
        ));
    }

    #[test]
    fn ignores_generic_coverage_and_out() {
        assert!(should_ignore_path(
            &PathBuf::from("/proj/coverage/lcov-report/index.html"),
            &root()
        ));
        assert!(should_ignore_path(
            &PathBuf::from("/proj/out/bundle.js"),
            &root()
        ));
    }

    #[test]
    fn does_not_ignore_bin_directory() {
        // `bin/` is too generic — some projects commit CLI shims there.
        // Never ignore it.
        assert!(!should_ignore_path(
            &PathBuf::from("/proj/bin/cli.ts"),
            &root()
        ));
    }

    #[test]
    fn does_not_ignore_src_files() {
        assert!(!should_ignore_path(
            &PathBuf::from("/proj/src/index.ts"),
            &root()
        ));
    }

    #[test]
    fn does_not_ignore_root_dotfiles() {
        // .env.local, .iii.worker.yaml etc. should trigger restart.
        assert!(!should_ignore_path(
            &PathBuf::from("/proj/.env.local"),
            &root()
        ));
    }

    #[test]
    fn does_not_ignore_file_named_like_artifact_dir() {
        // `node_modules.md` is not `node_modules`.
        assert!(!should_ignore_path(
            &PathBuf::from("/proj/docs/node_modules.md"),
            &root()
        ));
    }

    #[test]
    fn ignores_when_project_root_prefix_does_not_match() {
        // If notify emits an absolute path outside the root (shouldn't
        // normally happen, but be defensive), we still filter by name.
        assert!(should_ignore_path(
            &PathBuf::from("/other/node_modules/pkg/foo.js"),
            &root()
        ));
    }

    /// Regression lock for the `starts_with(root)` bounds check added
    /// at `register_new_dirs` (line ~328). If a path outside the
    /// canonical project root sneaks through the earlier filters
    /// (ignore-set + symlink-metadata), the bounds check MUST still
    /// reject it before `watcher.watch()` is called — otherwise a
    /// future notify-layer refactor could surface cross-root event
    /// paths (e.g. FSEvents resolving a mount alias) and silently
    /// register watches outside the project tree.
    #[test]
    fn register_new_dirs_rejects_out_of_root_paths() {
        // Two independent tempdirs. `root` is the project; `stranger`
        // simulates any path notify might hand us that is a real
        // directory but outside the tree we meant to watch.
        let proj = tempfile::tempdir().unwrap();
        let stranger_tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(proj.path()).unwrap();
        let stranger = std::fs::canonicalize(stranger_tmp.path()).unwrap();

        // Sanity: stranger is NOT under root (the whole point of the
        // fixture). If this fires the tempdir layout changed.
        assert!(!stranger.starts_with(&root));

        let mut watcher = notify::recommended_watcher(|_| {}).unwrap();
        let mut registered: HashSet<PathBuf> = HashSet::new();
        register_new_dirs(
            &mut watcher,
            std::slice::from_ref(&stranger),
            &root,
            &mut registered,
        );

        assert!(
            !registered.contains(&stranger),
            "out-of-root path must be rejected by the bounds check; \
             if this fires, the `starts_with(root)` guard at \
             source_watcher.rs:~328 was weakened or removed"
        );
        assert!(
            registered.is_empty(),
            "no watches should be registered for an out-of-root burst"
        );
    }

    /// Complement: an in-root path (real directory, not ignored) must
    /// still register. Guards against the bounds check being
    /// accidentally inverted or over-tightened.
    #[test]
    fn register_new_dirs_accepts_in_root_paths() {
        let proj = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(proj.path()).unwrap();
        let child = root.join("src");
        std::fs::create_dir_all(&child).unwrap();

        let mut watcher = notify::recommended_watcher(|_| {}).unwrap();
        let mut registered: HashSet<PathBuf> = HashSet::new();
        register_new_dirs(
            &mut watcher,
            std::slice::from_ref(&child),
            &root,
            &mut registered,
        );

        assert!(
            registered.contains(&child),
            "in-root directory must register; got registered set: {:?}",
            registered
        );
    }

    /// Regression lock for the canonical-root seed: `watch_pruned` is
    /// called with `&root_for_filter` (canonicalized) rather than the
    /// raw user-supplied `project_path`, so the paths inserted into
    /// `registered_dirs` match the canonical form that notify's
    /// per-burst `register_new_dirs` lookup expects. On macOS FSEvents
    /// always emits canonical `/private/var/...` event paths even when
    /// the watch was registered on a symlinked ancestor; seeding with
    /// the raw (non-canonical) path would make every burst miss the
    /// dedup-set and redo `lstat` + `inotify_add_watch` on parents we
    /// already watched.
    ///
    /// Strategy: walk a canonicalized tempdir tree, then verify that
    /// `registered_dirs` contains the canonical root path (not a
    /// symlinked alias). On filesystems where tempdir() paths are
    /// already canonical this degenerates to a presence check — still
    /// useful as a regression fence against any refactor that seeds
    /// with a non-canonical argument.
    #[test]
    fn watch_pruned_seeds_registered_with_canonical_root() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(tmp.path()).unwrap();

        let mut watcher = notify::recommended_watcher(|_| {}).unwrap();
        let mut registered: HashSet<PathBuf> = HashSet::new();
        watch_pruned(&mut watcher, &canonical_root, &mut registered).unwrap();

        assert!(
            registered.contains(&canonical_root),
            "watch_pruned must seed `registered` with the canonical root \
             so burst-time lookups from notify hit the dedup set; \
             got registered set: {:?}",
            registered
        );
    }
}
