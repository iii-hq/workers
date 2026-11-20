//! The session follows a git worktree its own `shell::exec` creates.
//!
//! Around a host `shell::exec` whose command line names `worktree`, the turn
//! hooks diff the git worktree registry of the stamped root. One new (or the
//! session's own, moved) worktree named on the command line, or else the
//! stamped worktree removed (the way back to the main checkout), is answered
//! to the harness as
//! `mutations.fs_scope.root`, which moves the session there. The registry is
//! the proof, not argv parsing or the exit code.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::config::{ExecConfinement, ShellConfig};
use crate::exec::confine::{linked_worktrees, repo_layout};
use crate::fs::{FsBoundary, FsScope};
use crate::turns::HookInput;

/// Bounds for snapshots whose post-call never comes (a hold, a deny, a hook
/// timeout).
const MAX_AGE: Duration = Duration::from_secs(3_600);
const MAX_SNAPS: usize = 256;

/// `(session_id, turn_id, call_id)`.
type Key = (String, String, String);

struct Snap {
    root_raw: String,
    root: PathBuf,
    top: PathBuf,
    common: PathBuf,
    /// Admin dir to worktree top, see [`linked_worktrees`].
    before: BTreeMap<PathBuf, PathBuf>,
    at: Instant,
}

#[derive(Default)]
pub struct WorktreeFollow {
    snaps: Mutex<HashMap<Key, Snap>>,
}

/// A host `shell::exec` with a stamped scope and `worktree` on its command
/// line.
struct Exec {
    key: Key,
    scope: FsScope,
    line: String,
}

// shortcut: only `shell::exec` is watched; an `exec_bg` job or a PTY session
// never moves its session, upgrade when agents create worktrees there.
fn exec_of(input: &HookInput) -> Option<Exec> {
    let call = input
        .call
        .as_ref()
        .filter(|c| c.function_id == "shell::exec")?;
    let args = &call.arguments;
    let host = args
        .get("target")
        .filter(|t| !t.is_null())
        .is_none_or(|t| t.get("kind").and_then(Value::as_str) == Some("host"));
    let scope: FsScope = serde_json::from_value(args.get("fs_scope")?.clone()).ok()?;
    let mut line = args.get("command")?.as_str()?.to_string();
    for arg in args
        .get("args")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        line.push(' ');
        line.push_str(arg.as_str().unwrap_or_default());
    }
    let key = (
        input.session_id.clone()?,
        input.turn_id.clone()?,
        call.id.clone(),
    );
    (host && line.contains("worktree")).then_some(Exec { key, scope, line })
}

impl WorktreeFollow {
    fn snaps(&self) -> MutexGuard<'_, HashMap<Key, Snap>> {
        self.snaps.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Snapshot the worktree registry of the root the exec runs under.
    pub async fn on_pre(&self, input: &HookInput) {
        let Some(exec) = exec_of(input) else {
            return;
        };
        let root_raw = exec.scope.root;
        let snap = tokio::task::spawn_blocking(move || {
            let root = Path::new(&root_raw).canonicalize().ok()?;
            let (top, common) = repo_layout(&root)?;
            let before = linked_worktrees(&common);
            Some(Snap {
                root_raw,
                root,
                top,
                common,
                before,
                at: Instant::now(),
            })
        })
        .await
        .ok()
        .flatten();
        let Some(snap) = snap else {
            return;
        };
        let mut snaps = self.snaps();
        snaps.retain(|_, s| s.at.elapsed() < MAX_AGE);
        // shortcut: past MAX_SNAPS pending snapshots a worktree exec does not
        // move its session, upgrade if held calls ever pile up that far.
        if snaps.len() < MAX_SNAPS {
            snaps.insert(exec.key, snap);
        }
    }

    /// The root the session moves to after this exec, if any.
    pub async fn on_post(
        &self,
        input: &HookInput,
        cfg: std::sync::Arc<ShellConfig>,
    ) -> Option<String> {
        let exec = exec_of(input)?;
        let snap = self.snaps().remove(&exec.key)?;
        // A root changed while the call was held is not the one snapshotted.
        if input.result.as_ref().is_none_or(|r| r.is_error) || exec.scope.root != snap.root_raw {
            return None;
        }
        tokio::task::spawn_blocking(move || moved_root(&snap, &exec, &cfg))
            .await
            .ok()
            .flatten()
    }
}

fn moved_root(snap: &Snap, exec: &Exec, cfg: &ShellConfig) -> Option<String> {
    let main = snap.common.parent()?;
    // A new admin id, or the session's own worktree at a new path; another
    // worktree moved (`git worktree move`/`repair`) was not created here.
    // The name is required even for a single entry, so a failed add never
    // adopts a worktree another session created meanwhile.
    // shortcut: a directory named only through a shell variable or a script
    // does not move the session, upgrade if agents rely on that.
    let named: Vec<PathBuf> = linked_worktrees(&snap.common)
        .into_iter()
        .filter(|(id, t)| {
            snap.before
                .get(id)
                .is_none_or(|was| *was == snap.top && *t != snap.top)
        })
        .map(|(_, t)| t)
        .filter(|t| {
            t.file_name()
                .is_some_and(|n| exec.line.contains(&*n.to_string_lossy()))
        })
        .collect();
    let top = match named.as_slice() {
        [top] => top.clone(),
        // The linked worktree the session sat in is gone (not only the
        // root's subdirectory) and nothing replaced it: back to the main
        // checkout.
        [] if !snap.top.is_dir()
            && repo_layout(main) == Some((main.to_path_buf(), snap.common.clone())) =>
        {
            main.to_path_buf()
        }
        _ => return None,
    };
    // The same subdirectory, unless it resolves out of the worktree.
    let sub = snap.root.strip_prefix(&snap.top).ok()?;
    let cand = top
        .join(sub)
        .canonicalize()
        .ok()
        .filter(|c| c.is_dir() && c.starts_with(&top))
        .unwrap_or(top);
    let cand = crate::functions::workspace::canonical_workspace_dir(cand.to_str()?, cfg).ok()?;
    if cand == snap.root {
        return None;
    }
    let (host_roots, denylist) = crate::exec::policy::jail_inputs(cfg).ok()?;
    // Under Landlock the move goes only where the exec itself could write.
    if cfg.fs.exec_confinement == ExecConfinement::Landlock {
        let grants = crate::fs::host::confine_scope_grants(exec.scope.grants(), &denylist);
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let writable = crate::exec::confine::writable_roots(
            &snap.root,
            &grants,
            &cfg.fs.exec_writable,
            home.as_deref(),
        );
        if !writable.iter().any(|r| cand.starts_with(r)) {
            return None;
        }
    }
    // Never move where relative `coder::*` paths would be refused.
    if exec.scope.boundary == FsBoundary::ConfiguredRoots
        && crate::fs::host::confine_path(cand.to_str()?, &host_roots, &denylist).is_err()
    {
        return None;
    }
    Some(cand.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// `<tmp>/main`: one commit holding `sub/f`.
    fn repo() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().canonicalize().unwrap().join("main");
        std::fs::create_dir_all(main.join("sub")).unwrap();
        std::fs::write(main.join("sub/f"), "x").unwrap();
        git(&main, &["init", "-q"]);
        git(&main, &["add", "."]);
        git(&main, &["commit", "-q", "-m", "init"]);
        (tmp, main)
    }

    fn input(call_id: &str, turn: &str, root: &Path, argv: &[&str], scope: Value) -> HookInput {
        let mut fs_scope = json!({ "root": root, "grants": [], "boundary": "workspace" });
        for (k, v) in scope.as_object().into_iter().flatten() {
            fs_scope[k] = v.clone();
        }
        serde_json::from_value(json!({
            "session_id": "s_1",
            "turn_id": turn,
            "call": {
                "id": call_id,
                "function_id": "shell::exec",
                "arguments": { "command": argv[0], "args": &argv[1..], "fs_scope": fs_scope },
            },
            "result": { "is_error": false, "details": {} },
        }))
        .unwrap()
    }

    /// What the exec does between the hooks; its exit status is ignored, as
    /// the hook ignores it.
    fn run(cwd: &Path, argv: &[&str]) {
        std::process::Command::new(argv[0])
            .args(&argv[1..])
            .current_dir(cwd)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
    }

    async fn follow_with(
        cfg: ShellConfig,
        root: &Path,
        argv: &[&str],
        scope: Value,
    ) -> Option<String> {
        let f = WorktreeFollow::default();
        let hook = input("c_1", "t_1", root, argv, scope);
        f.on_pre(&hook).await;
        run(root, argv);
        f.on_post(&hook, Arc::new(cfg)).await
    }

    async fn follow(root: &Path, argv: &[&str]) -> Option<String> {
        follow_with(ShellConfig::default(), root, argv, json!({})).await
    }

    fn s(p: PathBuf) -> Option<String> {
        Some(p.to_string_lossy().into_owned())
    }

    #[tokio::test]
    async fn follows_a_new_worktree_its_command_names() {
        let (tmp, main) = repo();
        let dir = tmp.path().canonicalize().unwrap();
        assert_eq!(
            follow(&main, &["git", "worktree", "add", "-q", "../a"]).await,
            s(dir.join("a"))
        );
        // Through a shell, and a failing second half still follows.
        assert_eq!(
            follow(&main, &["sh", "-c", "git worktree add -q ../b && false"]).await,
            s(dir.join("b"))
        );
        let c = dir.join("c");
        let c_arg = c.to_str().unwrap();
        let main_arg = main.to_str().unwrap();
        assert_eq!(
            follow(
                &main,
                &["git", "-C", main_arg, "worktree", "add", "-q", c_arg]
            )
            .await,
            s(c)
        );
        // A subdirectory root maps to the same subdirectory.
        assert_eq!(
            follow(
                &main.join("sub"),
                &["git", "worktree", "add", "-q", "../../d"]
            )
            .await,
            s(dir.join("d/sub"))
        );
    }

    #[tokio::test]
    async fn no_move_without_a_new_named_worktree() {
        let (tmp, main) = repo();
        let dir = tmp.path().canonicalize().unwrap();
        // git fails: the target is a non-empty directory.
        std::fs::create_dir_all(dir.join("full/x")).unwrap();
        assert_eq!(
            follow(&main, &["git", "worktree", "add", "../full"]).await,
            None
        );
        // The new worktree's name is not on the command line.
        assert_eq!(
            follow(
                &main,
                &["sh", "-c", "git worktree add -q ../$(echo side | rev)"]
            )
            .await,
            None
        );
        assert!(dir.join("edis").is_dir());
        // A worktree of another repository.
        let other = dir.join("other");
        std::fs::create_dir_all(&other).unwrap();
        git(&other, &["init", "-q"]);
        git(&other, &["commit", "-q", "--allow-empty", "-m", "init"]);
        let other_arg = other.to_str().unwrap();
        assert_eq!(
            follow(
                &main,
                &["git", "-C", other_arg, "worktree", "add", "-q", "../ox"]
            )
            .await,
            None
        );
        // A denylisted target.
        let mut cfg = ShellConfig::default();
        cfg.fs.denylist_paths = vec![dir.join("denied")];
        assert_eq!(
            follow_with(
                cfg,
                &main,
                &["git", "worktree", "add", "-q", "../denied"],
                json!({})
            )
            .await,
            None
        );
        // An errored call.
        let f = WorktreeFollow::default();
        let mut hook = input(
            "c_1",
            "t_1",
            &main,
            &["git", "worktree", "add", "-q", "../e"],
            json!({}),
        );
        f.on_pre(&hook).await;
        run(&main, &["git", "worktree", "add", "-q", "../e"]);
        hook.result.as_mut().unwrap().is_error = true;
        assert_eq!(
            f.on_post(&hook, Arc::new(ShellConfig::default())).await,
            None
        );
    }

    #[tokio::test]
    async fn parallel_calls_each_adopt_the_worktree_they_named() {
        let (tmp, main) = repo();
        let dir = tmp.path().canonicalize().unwrap();
        let f = WorktreeFollow::default();
        let cfg = Arc::new(ShellConfig::default());
        let one = ["git", "worktree", "add", "-q", "../one"];
        let two = ["git", "worktree", "add", "-q", "../two"];
        let (h1, h2) = (
            input("c_1", "t_1", &main, &one, json!({})),
            input("c_2", "t_1", &main, &two, json!({})),
        );
        f.on_pre(&h1).await;
        f.on_pre(&h2).await;
        run(&main, &one);
        run(&main, &two);
        assert_eq!(f.on_post(&h2, cfg.clone()).await, s(dir.join("two")));
        assert_eq!(f.on_post(&h1, cfg).await, s(dir.join("one")));
    }

    /// `other`'s `sub` is a symlink out of the repository.
    #[tokio::test]
    async fn a_symlinked_subpath_falls_back_to_the_worktree_top() {
        let (tmp, main) = repo();
        let dir = tmp.path().canonicalize().unwrap();
        std::fs::create_dir_all(dir.join("outside")).unwrap();
        git(&main, &["checkout", "-q", "-b", "other"]);
        std::fs::remove_dir_all(main.join("sub")).unwrap();
        std::os::unix::fs::symlink("../outside", main.join("sub")).unwrap();
        git(&main, &["add", "-A"]);
        git(&main, &["commit", "-q", "-m", "link"]);
        git(&main, &["checkout", "-q", "-"]);
        assert_eq!(
            follow(
                &main.join("sub"),
                &["git", "worktree", "add", "-q", "../../l", "other"]
            )
            .await,
            s(dir.join("l"))
        );
    }

    #[tokio::test]
    async fn landlock_and_configured_roots_bound_the_move() {
        let (tmp, main) = repo();
        let dir = tmp.path().canonicalize().unwrap();
        let mut landlock = ShellConfig::default();
        landlock.fs.exec_confinement = ExecConfinement::Landlock;
        // The default exec_writable holds /tmp, where the tempdir may live.
        landlock.fs.exec_writable = Vec::new();
        // A sibling worktree hidden from the registry, restored by the exec.
        git(&main, &["worktree", "add", "-q", "../old"]);
        let admin = main.join(".git/worktrees/old");
        let hidden = main.join(".git/hidden-old");
        std::fs::rename(&admin, &hidden).unwrap();
        let restore = format!("mv {} {} # old", hidden.display(), admin.display());
        let argv = ["sh", "-c", restore.as_str()];
        assert_eq!(
            follow_with(landlock.clone(), &main, &argv, json!({})).await,
            None
        );
        // Unconfined, the exec could write there anyway: the name decides.
        std::fs::rename(&admin, &hidden).unwrap();
        assert_eq!(follow(&main, &argv).await, s(dir.join("old")));
        // In the root, or in a grant, it follows.
        assert_eq!(
            follow_with(
                landlock.clone(),
                &main,
                &["git", "worktree", "add", "-q", ".worktrees/in"],
                json!({})
            )
            .await,
            s(main.join(".worktrees/in"))
        );
        std::fs::create_dir_all(dir.join("granted")).unwrap();
        assert_eq!(
            follow_with(
                landlock,
                &main,
                &["git", "worktree", "add", "-q", "../granted/g"],
                json!({ "grants": [dir.join("granted")] })
            )
            .await,
            s(dir.join("granted/g"))
        );
        // Under configured_roots the move stays inside fs.host_roots.
        let mut jailed = ShellConfig::default();
        jailed.fs.host_roots = vec![main.clone()];
        let roots = json!({ "boundary": "configured_roots" });
        assert_eq!(
            follow_with(
                jailed.clone(),
                &main,
                &["git", "worktree", "add", "-q", "../out"],
                roots.clone()
            )
            .await,
            None
        );
        assert_eq!(
            follow_with(
                jailed,
                &main,
                &["git", "worktree", "add", "-q", ".worktrees/j"],
                roots
            )
            .await,
            s(main.join(".worktrees/j"))
        );
    }

    #[tokio::test]
    async fn removing_the_root_worktree_returns_to_the_main_checkout() {
        let (tmp, main) = repo();
        let feat = tmp.path().canonicalize().unwrap().join("feat");
        git(&main, &["worktree", "add", "-q", feat.to_str().unwrap()]);
        let feat_arg = feat.to_str().unwrap();
        let main_arg = main.to_str().unwrap();
        assert_eq!(
            follow(
                &feat.join("sub"),
                &[
                    "git", "-C", main_arg, "worktree", "remove", "--force", feat_arg
                ]
            )
            .await,
            s(main.join("sub"))
        );
        // Under Landlock only when the exec could write the main checkout.
        let mut landlock = ShellConfig::default();
        landlock.fs.exec_confinement = ExecConfinement::Landlock;
        landlock.fs.exec_writable = Vec::new();
        for (writable, expected) in [(false, None), (true, s(main.clone()))] {
            let wt = tmp
                .path()
                .canonicalize()
                .unwrap()
                .join(format!("ll-{writable}"));
            git(&main, &["worktree", "add", "-q", wt.to_str().unwrap()]);
            if writable {
                landlock.fs.exec_writable = vec![tmp.path().to_string_lossy().into_owned()];
            }
            let argv = ["git", "-C", main_arg, "worktree", "remove", "--force"];
            let argv = [&argv[..], &[wt.to_str().unwrap()][..]].concat();
            assert_eq!(
                follow_with(landlock.clone(), &wt, &argv, json!({})).await,
                expected
            );
        }
        // Only the root's subdirectory is gone: the session stays.
        let other = tmp.path().canonicalize().unwrap().join("other");
        git(&main, &["worktree", "add", "-q", other.to_str().unwrap()]);
        assert_eq!(
            follow(
                &other.join("sub"),
                &["sh", "-c", "rm -rf ../sub # worktree"]
            )
            .await,
            None
        );
        assert!(other.is_dir() && !other.join("sub").exists());
    }

    #[tokio::test]
    async fn a_moved_worktree_counts_only_when_it_is_the_sessions_own() {
        let (tmp, main) = repo();
        let dir = tmp.path().canonicalize().unwrap();
        let main_arg = main.to_str().unwrap();
        let [other, other2, feat, feat2, f4, nxt] =
            ["other", "other2", "feat", "feat2", "f4", "nxt"]
                .map(|n| dir.join(n).to_string_lossy().into_owned());
        let mv = ["git", "-C", main_arg, "worktree", "move"];
        // Moving another worktree is not creating one.
        git(&main, &["worktree", "add", "-q", &other]);
        let argv = [&mv[..], &[other.as_str(), &other2][..]].concat();
        assert_eq!(follow(&main, &argv).await, None);
        // The session's own worktree moved: it goes along.
        git(&main, &["worktree", "add", "-q", &feat]);
        let argv = [&mv[..], &[feat.as_str(), &feat2][..]].concat();
        assert_eq!(
            follow(&dir.join("feat/sub"), &argv).await,
            s(dir.join("feat2/sub"))
        );
        // Removed and replaced in one exec: the new worktree, not the main
        // checkout.
        git(&main, &["worktree", "add", "-q", &f4]);
        let swap = format!(
            "git -C {main_arg} worktree remove --force {f4} && git -C {main_arg} worktree add -q {nxt}"
        );
        assert_eq!(
            follow(&dir.join("f4"), &["sh", "-c", &swap]).await,
            s(dir.join("nxt"))
        );
    }

    #[tokio::test]
    async fn snapshots_are_keyed_by_turn_and_capped() {
        let (_tmp, main) = repo();
        let f = WorktreeFollow::default();
        let argv = ["git", "worktree", "list"];
        f.on_pre(&input("c_1", "t_1", &main, &argv, json!({})))
            .await;
        f.on_pre(&input("c_1", "t_2", &main, &argv, json!({})))
            .await;
        assert_eq!(f.snaps().len(), 2);
        for i in 0..MAX_SNAPS {
            f.on_pre(&input(&format!("c_{i}"), "t_3", &main, &argv, json!({})))
                .await;
        }
        assert_eq!(f.snaps().len(), MAX_SNAPS);
        // Neither the plain `git status` nor a sandbox exec is snapshotted.
        let f = WorktreeFollow::default();
        f.on_pre(&input("c_1", "t_1", &main, &["git", "status"], json!({})))
            .await;
        // An explicit `"target": null` is the host.
        let mut null_target = input("c_3", "t_1", &main, &argv, json!({}));
        null_target.call.as_mut().unwrap().arguments["target"] = Value::Null;
        f.on_pre(&null_target).await;
        assert_eq!(f.snaps().len(), 1);
        f.snaps().clear();
        let mut sandboxed = input("c_2", "t_1", &main, &argv, json!({}));
        sandboxed.call.as_mut().unwrap().arguments["target"] =
            json!({ "kind": "sandbox", "sandbox_id": uuid::Uuid::nil() });
        f.on_pre(&sandboxed).await;
        assert!(f.snaps().is_empty());
    }
}
