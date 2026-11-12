//! Claude Code accounts against a real tmux on a private socket
//! (`-L iii-tmux-acct-<uuid>`) with the demo kit's fake Claude Code and a
//! temporary home: never the developer's sessions, environment or sign-in.
//! Skips when tmux or python3 is missing. Every scenario kills its server
//! and removes its folder at the end, pass or fail.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tmux::accounts::credentials::FileStore;
use tmux::accounts::paths::ClaudePaths;
use tmux::accounts::Accounts;
use tmux::collector::{Collector, ConsoleClients};
use tmux::config::WorkerConfig;
use tmux::events::Monitor;
use tmux::functions::accounts::{self as account_fns, ClaudeLaunchInput};
use tmux::functions::{self, panes, sessions, windows, Ctx, EmptyInput};
use tmux::model::{Follow, Pane, PaneClaude};

struct Sandbox {
    dir: PathBuf,
    socket: String,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::process::Command::new("tmux")
            .args(["-L", &self.socket, "kill-server"])
            .env_remove("TMUX")
            .output();
        let tmpdir = std::env::var("TMUX_TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
        // SAFETY: getuid has no preconditions and cannot fail.
        let uid = unsafe { libc::getuid() };
        let _ = std::fs::remove_file(format!("{tmpdir}/tmux-{uid}/{}", self.socket));
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn available() -> bool {
    let runs = |program: &str, arg: &str| {
        std::process::Command::new(program)
            .arg(arg)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    };
    runs("tmux", "-V") && runs("python3", "--version")
}

struct World {
    sandbox: Sandbox,
    ctx: Ctx,
    collector: Arc<Collector>,
    paths: ClaudePaths,
}

impl World {
    fn home(&self) -> PathBuf {
        self.sandbox.dir.join("home")
    }

    fn accounts(&self) -> &Arc<Accounts> {
        self.collector.accounts()
    }

    /// `tmux -L <socket> args…` on this world's server.
    fn tmux(&self, args: &[&str]) -> std::process::Output {
        std::process::Command::new("tmux")
            .arg("-L")
            .arg(&self.sandbox.socket)
            .args(args)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .output()
            .expect("tmux runs")
    }

    fn tmux_out(&self, args: &[&str]) -> String {
        let output = self.tmux(args);
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    async fn pane(&self, pane_id: &str) -> Option<Pane> {
        let snapshot = functions::sessions_list(&self.ctx, EmptyInput {})
            .await
            .ok()?;
        snapshot
            .sessions
            .into_iter()
            .flat_map(|session| session.windows)
            .flat_map(|window| window.panes)
            .find(|pane| pane.id == pane_id)
    }

    /// The pane's Claude Code once it has recorded its session.
    async fn claude(&self, pane_id: &str) -> PaneClaude {
        eventually(&format!("Claude Code in {pane_id}"), || async {
            self.pane(pane_id)
                .await?
                .claude
                .filter(|claude| claude.session_id.is_some())
        })
        .await
    }
}

/// A world whose tmux server runs with a temporary home and the fake Claude
/// Code first on its PATH, started here so its global environment is that.
fn world(name: &str) -> World {
    let dir = std::env::temp_dir().join(format!(
        "tmux-acct-live-{name}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let socket = format!("iii-tmux-acct-{}", uuid::Uuid::new_v4().simple());
    let sandbox = Sandbox {
        dir: dir.clone(),
        socket: socket.clone(),
    };
    let home = dir.join("home");
    let bin = dir.join("bin");
    for folder in [
        home.join(".claude/projects"),
        bin.clone(),
        dir.join("state"),
    ] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("demo/files/claude");
    std::fs::copy(&fake, bin.join("claude")).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(bin.join("claude"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }
    let vars: HashMap<&str, String> = HashMap::from([
        ("HOME", home.to_string_lossy().into_owned()),
        (
            "XDG_CACHE_HOME",
            dir.join("cache").to_string_lossy().into_owned(),
        ),
        (
            "III_CLAUDE_ACCOUNTS_DIR",
            dir.join("accounts").to_string_lossy().into_owned(),
        ),
        ("III_CLAUDE_USAGE_URL", "http://127.0.0.1:9".to_string()),
        (
            "CODEX_HOME",
            dir.join("codex").to_string_lossy().into_owned(),
        ),
    ]);
    let paths = ClaudePaths::from_vars(|key| vars.get(key).cloned(), false).unwrap();
    let accounts = Arc::new(Accounts::new(
        Some(paths.clone()),
        Box::new(FileStore),
        false,
    ));
    // No test reads this computer's sign-in or asks the real endpoint (O2).
    assert!(paths.claude_home.starts_with(&dir) && paths.default_json.starts_with(&dir));
    assert_eq!(accounts.usage_origin(), Some("http://127.0.0.1:9"));
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    // tmux gives a window made from outside tmux the PATH of the command
    // that asked for it, so the worker's tmux runs with the fake first too.
    let real_tmux = which("tmux");
    let wrapper = dir.join("tmux");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nPATH={}\nexport PATH\nexec {} \"$@\"\n",
            sh_quote(&path),
            sh_quote(&real_tmux.to_string_lossy())
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // /bin/sh runs the panes' commands: a developer's zsh setup (ZDOTDIR)
    // could put their real `claude` back first on the PATH.
    let started = std::process::Command::new("tmux")
        .args(["-L", &socket, "-f", "/dev/null", "new-session", "-d"])
        .args(["-s", "base", "-x", "200", "-y", "50", "/bin/sh"])
        .args([";", "set-option", "-g", "default-shell", "/bin/sh"])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("ZDOTDIR")
        .env_remove("ENV")
        .env_remove("BASH_ENV")
        .env("SHELL", "/bin/sh")
        .env("HOME", &home)
        .env("PATH", path)
        .env("FAKE_CLAUDE_STATE_DIR", dir.join("state"))
        // The fake refuses to run outside a folder that holds all it writes.
        .env("FAKE_CLAUDE_SANDBOX", &dir)
        .current_dir(&home)
        .output()
        .unwrap();
    assert!(started.status.success(), "{started:?}");
    // Never the real Claude Code: `claude` in a pane must be the fake one.
    let which = dir.join("which");
    let probe = std::process::Command::new(&wrapper)
        .args([
            "-L",
            &socket,
            "new-window",
            "-d",
            "-t",
            "base:",
            "-n",
            "probe",
        ])
        .arg(format!("command -v claude > '{}'", which.display()))
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert!(probe.status.success(), "{probe:?}");
    let deadline = Instant::now() + Duration::from_secs(5);
    while std::fs::read_to_string(&which).map_or(true, |text| text.trim().is_empty()) {
        assert!(Instant::now() < deadline, "the probe window never ran");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        std::fs::read_to_string(&which).unwrap().trim(),
        bin.join("claude").to_string_lossy(),
        "a pane would run another claude"
    );
    let config = WorkerConfig {
        socket_name: Some(socket),
        tmux_binary: wrapper.to_string_lossy().into_owned(),
        ..WorkerConfig::default()
    }
    .into_shared();
    let collector = Arc::new(Collector::with_accounts(
        config,
        ConsoleClients::default(),
        accounts,
    ));
    let monitor = Monitor::new(
        iii_sdk::IIIClient::new("ws://stub-not-connected:0"),
        collector.clone(),
    );
    World {
        sandbox,
        ctx: Ctx {
            collector: collector.clone(),
            monitor,
            transcripts: Default::default(),
        },
        collector,
        paths,
    }
}

async fn eventually<T, F, Fut>(what: &str, mut probe: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(value) = probe().await {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

fn claude(account: &str, skip_permissions: bool) -> Option<ClaudeLaunchInput> {
    Some(ClaudeLaunchInput {
        account: account.to_string(),
        skip_permissions,
    })
}

fn window(name: &str, launch: Option<ClaudeLaunchInput>) -> windows::CreateWindowInput {
    windows::CreateWindowInput {
        session: "base".into(),
        name: Some(name.into()),
        cwd: None,
        command: None,
        select: Some(false),
        remain_on_exit: false,
        claude: launch,
    }
}

#[tokio::test]
async fn claude_starts_on_the_account_asked_for() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("create");
    let personal = account_fns::add(
        &w.ctx,
        account_fns::AddAccountInput {
            name: "Personal".into(),
            color: "green".into(),
            shared: Some(vec!["conversations".into()]),
        },
    )
    .await
    .unwrap()
    .account;
    assert_eq!(personal.id, "personal");
    let personal_dir = w.paths.account_dir("personal");

    // Pinned to an account, skipping permission prompts (refused as root,
    // where Claude Code refuses it: then without).
    let skip = can_skip(&w).await;
    let pinned = windows::create(&w.ctx, window("pinned", claude("personal", skip)))
        .await
        .unwrap();
    assert_eq!(
        pinned.runs.as_deref(),
        Some(
            format!(
                "env CLAUDE_CONFIG_DIR={} claude{}",
                personal_dir.display(),
                if skip {
                    " --dangerously-skip-permissions"
                } else {
                    ""
                }
            )
            .as_str()
        )
    );
    let info = w.claude(&pinned.pane_id).await;
    assert_eq!(info.account.as_deref(), Some("personal"));
    assert_eq!(info.follow, Follow::Pinned);
    assert_eq!(info.skip_permissions, skip);
    assert!(info.movable, "{info:?}");
    assert_eq!(
        w.tmux_out(&[
            "display",
            "-p",
            "-t",
            &pinned.pane_id,
            "#{@iii_claude_account}:#{@iii_skip_permissions}"
        ]),
        if skip { "personal:1" } else { "personal:" }
    );
    // Its conversation went to the account's folder, which shares
    // `projects/` with ~/.claude.
    let session_id = info.session_id.clone().unwrap();
    let transcripts: Vec<_> = walk(&w.home().join(".claude/projects"))
        .into_iter()
        .filter(|path| path.ends_with(format!("{session_id}.jsonl")))
        .collect();
    assert_eq!(transcripts.len(), 1, "shared through the link");

    // Following the global account: tmux's global environment says which.
    w.accounts()
        .update_registry(|registry| {
            registry.global = "personal".into();
            Ok(())
        })
        .unwrap();
    let follows = windows::create(&w.ctx, window("follows", claude("global", false)))
        .await
        .unwrap();
    assert_eq!(follows.runs.as_deref(), Some("claude"));
    assert_eq!(
        w.tmux_out(&["show-environment", "-g", "CLAUDE_CONFIG_DIR"]),
        format!("CLAUDE_CONFIG_DIR={}", personal_dir.display())
    );
    let info = w.claude(&follows.pane_id).await;
    assert_eq!(info.account.as_deref(), Some("personal"));
    assert_eq!(info.follow, Follow::Global);
    assert!(!info.skip_permissions);

    // A new session pinned to the default account, kept once it ends.
    let session = sessions::create(
        &w.ctx,
        sessions::CreateSessionInput {
            name: Some("work".into()),
            cwd: Some(w.home().to_string_lossy().into_owned()),
            command: None,
            window_name: Some("claude".into()),
            remain_on_exit: true,
            claude: claude("default", false),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        session.runs.as_deref(),
        Some("env -u CLAUDE_CONFIG_DIR claude")
    );
    let info = w.claude(&session.pane_id).await;
    assert_eq!(info.account.as_deref(), Some("default"));
    assert_eq!(info.follow, Follow::Pinned);
    let scope = if w.tmux_out(&["-V"]).starts_with("tmux 2.") {
        "-w"
    } else {
        "-p"
    };
    assert!(w
        .tmux_out(&[
            "show-options",
            scope,
            "-t",
            &session.pane_id,
            "remain-on-exit"
        ])
        .ends_with("on"));

    // Back to default: the global environment drops the variable.
    w.accounts()
        .update_registry(|registry| {
            registry.global = "default".into();
            Ok(())
        })
        .unwrap();
    let plain = windows::create(&w.ctx, window("plain", claude("global", false)))
        .await
        .unwrap();
    assert!(!w
        .tmux(&["show-environment", "-g", "CLAUDE_CONFIG_DIR"])
        .status
        .success());
    let info = w.claude(&plain.pane_id).await;
    assert_eq!(info.account.as_deref(), Some("default"));

    // A session's own CLAUDE_CONFIG_DIR wins over the global one in tmux: a
    // window there that follows the global account names it outright.
    let personal_text = personal_dir.to_string_lossy().into_owned();
    w.tmux(&[
        "set-environment",
        "-t",
        "base",
        "CLAUDE_CONFIG_DIR",
        &personal_text,
    ]);
    let named = windows::create(&w.ctx, window("named", claude("global", false)))
        .await
        .unwrap();
    assert_eq!(
        named.runs.as_deref(),
        Some("env -u CLAUDE_CONFIG_DIR claude")
    );
    let info = w.claude(&named.pane_id).await;
    assert_eq!(info.account.as_deref(), Some("default"));
    assert_eq!(info.follow, Follow::Global);
    w.tmux(&["set-environment", "-u", "-t", "base", "CLAUDE_CONFIG_DIR"]);

    // A folder is deleted only when nothing can still need it: never while
    // a pane is being moved.
    let moving = w.accounts().moving.begin("%999").unwrap();
    let refused = account_fns::remove(
        &w.ctx,
        account_fns::RemoveAccountInput {
            account: "personal".into(),
            delete_files: true,
        },
    )
    .await
    .unwrap_err();
    assert!(refused.contains("being moved"), "{refused}");
    drop(moving);

    // Refusals.
    let mut both = window("both", claude("global", false));
    both.command = Some("htop".into());
    let error = windows::create(&w.ctx, both).await.unwrap_err();
    assert!(error.contains("either"), "{error}");
    let error = windows::create(&w.ctx, window("nobody", claude("nobody", false)))
        .await
        .unwrap_err();
    assert!(error.contains("nobody"), "{error}");
}

/// `text` as one shell word, whatever it holds.
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

fn which(program: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join(program))
        .find(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(program))
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

impl World {
    /// The fake Claude Code's process in `pane`.
    async fn claude_pid(&self, pane: &str) -> u32 {
        eventually(&format!("Claude Code's pid in {pane}"), || async {
            self.collector
                .claude_panes(true)
                .await
                .get(pane)?
                .claude_pid
        })
        .await
    }

    /// Tell the fake Claude Code in `pane` to be `state` (working, waiting,
    /// idle), and wait until it says so in its record.
    async fn set_state(&self, pane: &str, state: &str) {
        let pid = self.claude_pid(pane).await;
        std::fs::write(
            self.sandbox.dir.join(format!("state/{pid}.state")),
            format!("{state}\n"),
        )
        .unwrap();
        let raw = match state {
            "working" => "busy",
            other => other,
        };
        eventually(&format!("{pane} {state}"), || async {
            let panes = self.collector.claude_panes(true).await;
            let status = panes.get(pane)?.record.as_ref()?.status.clone()?;
            (status == raw).then_some(())
        })
        .await;
    }

    /// The pane's Claude Code once it runs on `account` with no move waiting.
    async fn settled_on(&self, pane: &str, account: &str) -> PaneClaude {
        eventually(&format!("{pane} on {account}"), || async {
            self.pane(pane).await?.claude.filter(|claude| {
                claude.account.as_deref() == Some(account)
                    && claude.session_id.is_some()
                    && claude.pending.is_none()
            })
        })
        .await
    }

    async fn pending_tick(&self) {
        tmux::accounts::daemon::pending_tick(&self.collector, &self.ctx.monitor).await;
    }

    fn screen(&self, pane: &str) -> String {
        self.tmux_out(&["capture-pane", "-p", "-J", "-t", pane])
    }
}

/// Whether panes may skip permission prompts here: never as root, where
/// creating one is refused (checked).
async fn can_skip(w: &World) -> bool {
    if !tmux::accounts::runs_as_root() {
        return true;
    }
    let refused = windows::create(&w.ctx, window("root", claude("global", true)))
        .await
        .unwrap_err();
    assert!(refused.contains("root"), "{refused}");
    false
}

async fn pane_account(w: &World, pane: &str, account: &str) -> panes::PaneAccountOutput {
    panes::account(
        &w.ctx,
        panes::PaneAccountInput {
            pane: pane.into(),
            account: account.into(),
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn panes_move_between_accounts_between_turns() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("move");
    for name in ["Personal", "Side"] {
        account_fns::add(
            &w.ctx,
            account_fns::AddAccountInput {
                name: name.into(),
                color: "green".into(),
                // Nothing shared: a move has to copy the conversation.
                shared: Some(vec![]),
            },
        )
        .await
        .unwrap();
    }
    for dir in [
        w.paths.claude_home.clone(),
        w.paths.account_dir("personal"),
        w.paths.account_dir("side"),
    ] {
        signed_in(&dir);
    }
    let personal_dir = w.paths.account_dir("personal");

    // An idle pane that follows the global account moves at once, and
    // resumes its conversation there.
    let skip = can_skip(&w).await;
    let a = windows::create(&w.ctx, window("a", claude("global", skip)))
        .await
        .unwrap()
        .pane_id;
    let before = w.claude(&a).await;
    assert_eq!(before.account.as_deref(), Some("default"));
    let session = before.session_id.clone().unwrap();
    let moved = pane_account(&w, &a, "personal").await;
    assert_eq!(moved.state, panes::PaneAccountState::Moved);
    assert_eq!(moved.account, "personal");
    let after = w.settled_on(&a, "personal").await;
    assert_eq!(after.follow, Follow::Pinned);
    assert_eq!(
        after.session_id.as_deref(),
        Some(session.as_str()),
        "resumed"
    );
    assert_eq!(after.skip_permissions, skip, "skip kept");
    assert!(walk(&personal_dir.join("projects"))
        .iter()
        .any(|path| path.ends_with(format!("{session}.jsonl"))));
    assert!(w.screen(&a).contains("Resumed on the personal account"));
    let scope = if w.tmux_out(&["-V"]).starts_with("tmux 2.") {
        "-w"
    } else {
        "-p"
    };
    // Put back as it was: off, as the window was made.
    assert_eq!(
        w.tmux_out(&["show-options", scope, "-v", "-t", &a, "remain-on-exit"]),
        "off",
        "remain-on-exit put back"
    );

    // Pinning it where it runs changes no process.
    let pid = w.claude_pid(&a).await;
    assert_eq!(
        pane_account(&w, &a, "personal").await.state,
        panes::PaneAccountState::Pinned
    );
    assert_eq!(w.claude_pid(&a).await, pid);

    // Working: the move waits for the end of the turn.
    w.set_state(&a, "working").await;
    let waiting = pane_account(&w, &a, "global").await;
    assert_eq!(waiting.state, panes::PaneAccountState::AfterTurn);
    assert_eq!(waiting.account, "default");
    w.pending_tick().await;
    let info = w.pane(&a).await.unwrap().claude.unwrap();
    assert_eq!(info.pending.as_deref(), Some("default"));
    assert_eq!(info.account.as_deref(), Some("personal"));
    assert_eq!(w.claude_pid(&a).await, pid, "not moved while working");
    w.set_state(&a, "idle").await;
    w.pending_tick().await;
    let info = w.settled_on(&a, "default").await;
    assert_eq!(info.follow, Follow::Global);

    // Cancelling a waiting move: back to the account it runs on.
    w.set_state(&a, "waiting").await;
    assert_eq!(
        pane_account(&w, &a, "side").await.state,
        panes::PaneAccountState::AfterTurn
    );
    assert_eq!(
        pane_account(&w, &a, "default").await.state,
        panes::PaneAccountState::Cancelled
    );
    w.set_state(&a, "idle").await;
    w.pending_tick().await;
    let info = w.settled_on(&a, "default").await;
    assert_eq!(info.follow, Follow::Pinned);
    pane_account(&w, &a, "global").await;

    // Switching the global account: idle panes go now, working ones after.
    let b = windows::create(&w.ctx, window("b", claude("global", false)))
        .await
        .unwrap()
        .pane_id;
    w.claude(&b).await;
    w.set_state(&b, "working").await;
    let switched = account_fns::global(
        &w.ctx,
        account_fns::GlobalInput {
            account: "personal".into(),
            move_mode: account_fns::GlobalMove::IdleNowWorkingAfter,
        },
    )
    .await
    .unwrap();
    assert_eq!(switched.global, "personal");
    assert_eq!(switched.moved, vec![a.clone()]);
    assert_eq!(switched.after_turn, vec![b.clone()]);
    assert_eq!(
        w.tmux_out(&["show-environment", "-g", "CLAUDE_CONFIG_DIR"]),
        format!("CLAUDE_CONFIG_DIR={}", personal_dir.display())
    );
    assert_eq!(w.settled_on(&a, "personal").await.follow, Follow::Global);
    w.set_state(&b, "idle").await;
    w.pending_tick().await;
    assert_eq!(w.settled_on(&b, "personal").await.follow, Follow::Global);

    // New windows only: the others stay, pinned where they run.
    let switched = account_fns::global(
        &w.ctx,
        account_fns::GlobalInput {
            account: "default".into(),
            move_mode: account_fns::GlobalMove::NewOnly,
        },
    )
    .await
    .unwrap();
    let mut stayed = switched.stayed.clone();
    stayed.sort();
    let mut both = vec![a.clone(), b.clone()];
    both.sort();
    assert_eq!(stayed, both);
    assert!(switched.moved.is_empty() && switched.after_turn.is_empty());
    for pane in [&a, &b] {
        let info = w.settled_on(pane, "personal").await;
        assert_eq!(info.follow, Follow::Pinned);
    }
    assert!(!w
        .tmux(&["show-environment", "-g", "CLAUDE_CONFIG_DIR"])
        .status
        .success());

    // The palette's "move the idle ones": b is working, so only a goes.
    w.set_state(&b, "working").await;
    let moved = account_fns::move_idle(
        &w.ctx,
        account_fns::MoveIdleInput {
            from: "personal".into(),
            to: "side".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(moved.moved, vec![a.clone()]);
    let info = w.settled_on(&a, "side").await;
    assert_eq!(info.follow, Follow::Pinned);
    assert_eq!(
        w.pane(&b).await.unwrap().claude.unwrap().account.as_deref(),
        Some("personal")
    );
}

#[tokio::test]
async fn claude_started_by_hand_moves_through_its_shell() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("shell");
    account_fns::add(
        &w.ctx,
        account_fns::AddAccountInput {
            name: "Personal".into(),
            color: "green".into(),
            shared: None,
        },
    )
    .await
    .unwrap();
    signed_in(&w.paths.account_dir("personal"));
    let shell = windows::create(&w.ctx, window("shell", None))
        .await
        .unwrap()
        .pane_id;
    w.tmux(&["send-keys", "-t", &shell, "-l", "claude"]);
    w.tmux(&["send-keys", "-t", &shell, "Enter"]);
    let info = w.claude(&shell).await;
    assert_eq!(info.follow, Follow::Manual);
    assert_eq!(info.account.as_deref(), Some("default"));
    assert!(info.movable, "{info:?}");
    let session = info.session_id.clone().unwrap();
    let moved = pane_account(&w, &shell, "personal").await;
    assert_eq!(moved.state, panes::PaneAccountState::Moved);
    let info = w.settled_on(&shell, "personal").await;
    assert_eq!(info.follow, Follow::Pinned);
    assert_eq!(info.session_id.as_deref(), Some(session.as_str()));
    // The shell got the explicit command; it is still the pane's program.
    let screen = w.screen(&shell);
    assert!(
        screen.contains("Resumed on the personal account"),
        "{screen}"
    );
    let pane_pid = w.tmux_out(&["display", "-p", "-t", &shell, "#{pane_pid}"]);
    let claude_pid = w.claude_pid(&shell).await;
    assert_ne!(pane_pid, claude_pid.to_string());
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// An account folder signed in with a demo token: a move only goes to a
/// signed-in account.
fn signed_in(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join(".credentials.json"),
        serde_json::json!({"claudeAiOauth": {"accessToken": "demo-token",
            "expiresAt": now_ms() + 3_600_000, "subscriptionType": "pro"}})
        .to_string(),
    )
    .unwrap();
}

/// A signed-in account folder whose Claude Code cached a usage reading
/// (`cachedUsageUtilization` in its `.claude.json`): the week at `week` %,
/// resetting at `resets_ms`.
fn signed_in_with_usage(dir: &Path, json: &Path, week: f64, resets_ms: u64) {
    signed_in(dir);
    std::fs::write(
        json,
        serde_json::json!({
            "oauthAccount": {"emailAddress": "you@demo.dev"},
            "cachedUsageUtilization": {"fetchedAtMs": now_ms(), "utilization": {
                "seven_day": {"utilization": week, "resets_at": resets_ms}}}
        })
        .to_string(),
    )
    .unwrap();
}

#[tokio::test]
async fn rules_suggest_switch_and_switch_back() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("rules");
    account_fns::add(
        &w.ctx,
        account_fns::AddAccountInput {
            name: "Personal".into(),
            color: "green".into(),
            shared: Some(vec![]),
        },
    )
    .await
    .unwrap();
    // Room for a slow machine to get through the steps before the reset.
    let reset_at = now_ms() + 10_000;
    signed_in_with_usage(&w.paths.claude_home, &w.paths.default_json, 95.0, reset_at);
    signed_in_with_usage(
        &w.paths.account_dir("personal"),
        &w.paths.account_json("personal"),
        10.0,
        now_ms() + 86_400_000,
    );
    let a = windows::create(&w.ctx, window("a", claude("global", false)))
        .await
        .unwrap()
        .pane_id;
    w.claude(&a).await;

    // Suggest: a card, until "Not now".
    let rules = account_fns::rules(
        &w.ctx,
        account_fns::RulesInput {
            mode: Some(tmux::model::RulesMode::Suggest),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .rules;
    assert_eq!(rules.at, tmux::model::Threshold::Week90);
    let listed = account_fns::list(&w.ctx, EmptyInput {}).await.unwrap();
    let suggestion = listed.suggestion.expect("a suggestion");
    assert_eq!(
        (
            suggestion.from.as_str(),
            suggestion.to.as_str(),
            suggestion.reason.as_str(),
            suggestion.following
        ),
        ("default", "personal", "week_90", 1)
    );
    account_fns::dismiss(
        &w.ctx,
        account_fns::DismissInput {
            suggestion: suggestion.id.clone(),
        },
    )
    .await
    .unwrap();
    assert!(account_fns::list(&w.ctx, EmptyInput {})
        .await
        .unwrap()
        .suggestion
        .is_none());
    // An order naming no account is refused.
    assert!(account_fns::rules(
        &w.ctx,
        account_fns::RulesInput {
            order: Some(vec!["nobody".into()]),
            ..Default::default()
        },
    )
    .await
    .is_err());

    // Switch by itself, and back once the old week resets.
    account_fns::rules(
        &w.ctx,
        account_fns::RulesInput {
            mode: Some(tmux::model::RulesMode::Switch),
            back_after_reset: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(tmux::accounts::daemon::run_rules(&w.collector).await);
    let listed = account_fns::list(&w.ctx, EmptyInput {}).await.unwrap();
    assert_eq!(listed.global, "personal");
    let last = listed.last_switch.expect("last switch");
    assert!(last.auto);
    assert_eq!(
        (last.from.as_str(), last.to.as_str(), last.reason.as_deref()),
        ("default", "personal", Some("week_90"))
    );
    assert_eq!(last.moved, vec![a.clone()]);
    assert_eq!(last.back_at_ms, Some(reset_at));
    assert_eq!(w.settled_on(&a, "personal").await.follow, Follow::Global);
    assert!(
        !tmux::accounts::daemon::run_rules(&w.collector).await,
        "nothing more to do before the reset"
    );
    tokio::time::sleep(Duration::from_millis(
        reset_at.saturating_sub(now_ms()) + 200,
    ))
    .await;
    assert!(tmux::accounts::daemon::run_rules(&w.collector).await);
    let listed = account_fns::list(&w.ctx, EmptyInput {}).await.unwrap();
    assert_eq!(listed.global, "default");
    let last = listed.last_switch.unwrap();
    assert_eq!(
        (last.from.as_str(), last.to.as_str(), last.reason.as_deref()),
        ("personal", "default", Some("back"))
    );
    assert_eq!(w.settled_on(&a, "default").await.follow, Follow::Global);
}

#[tokio::test]
async fn a_person_switching_calls_off_the_switch_back() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("back");
    for name in ["Personal", "Side"] {
        account_fns::add(
            &w.ctx,
            account_fns::AddAccountInput {
                name: name.into(),
                color: "green".into(),
                shared: Some(vec![]),
            },
        )
        .await
        .unwrap();
    }
    let reset_at = now_ms() + 8_000;
    signed_in_with_usage(&w.paths.claude_home, &w.paths.default_json, 95.0, reset_at);
    for id in ["personal", "side"] {
        signed_in_with_usage(
            &w.paths.account_dir(id),
            &w.paths.account_json(id),
            10.0,
            now_ms() + 86_400_000,
        );
    }
    account_fns::rules(
        &w.ctx,
        account_fns::RulesInput {
            mode: Some(tmux::model::RulesMode::Switch),
            order: Some(vec!["personal".into()]),
            back_after_reset: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    // Nobody watches: nothing is read until the worker's own look at start
    // (accounts.json exists) finds another signed-in account (O1).
    assert!(!w.accounts().may_load(now_ms()));
    assert!(!tmux::accounts::daemon::run_rules(&w.collector).await);
    w.accounts().load_now().await;
    assert!(w.accounts().may_load(now_ms()));
    assert!(tmux::accounts::daemon::run_rules(&w.collector).await);
    let listed = account_fns::list(&w.ctx, EmptyInput {}).await.unwrap();
    assert_eq!(listed.global, "personal");
    assert_eq!(listed.last_switch.unwrap().back_at_ms, Some(reset_at));

    // Someone picks another account: the rules never undo that choice.
    account_fns::global(
        &w.ctx,
        account_fns::GlobalInput {
            account: "side".into(),
            move_mode: account_fns::GlobalMove::IdleNowWorkingAfter,
        },
    )
    .await
    .unwrap();
    let listed = account_fns::list(&w.ctx, EmptyInput {}).await.unwrap();
    assert_eq!(listed.last_switch.unwrap().back_at_ms, None);

    // The rules decided on a registry written since, even back to the same
    // values (side → personal → side): they switch nothing.
    let (decided_on, generation) = w.accounts().registry_at();
    for account in ["personal", "side"] {
        account_fns::global(
            &w.ctx,
            account_fns::GlobalInput {
                account: account.into(),
                move_mode: account_fns::GlobalMove::NewOnly,
            },
        )
        .await
        .unwrap();
    }
    assert_eq!(w.accounts().registry().global, decided_on.global);
    let tmux_now = w.collector.tmux().await.unwrap();
    let panes_now = w.collector.claude_panes(true).await;
    let stale = tmux::accounts::moves::switch_global(
        &tmux_now,
        w.accounts(),
        &panes_now,
        "personal",
        tmux::accounts::moves::SwitchMode::NewOnly,
        tmux::accounts::moves::SwitchBy::Rules {
            generation,
            record: tmux::model::AccountSwitch {
                from: "side".into(),
                to: "personal".into(),
                reason: Some("week_90".into()),
                auto: true,
                moved: Vec::new(),
                after_turn: Vec::new(),
                at_ms: now_ms(),
                back_at_ms: None,
            },
        },
    )
    .await
    .unwrap_err();
    assert_eq!(stale, tmux::accounts::moves::RULES_STALE);
    assert_eq!(w.accounts().registry().global, "side");
    tokio::time::sleep(Duration::from_millis(
        reset_at.saturating_sub(now_ms()) + 200,
    ))
    .await;
    assert!(!tmux::accounts::daemon::run_rules(&w.collector).await);
    assert_eq!(
        account_fns::list(&w.ctx, EmptyInput {})
            .await
            .unwrap()
            .global,
        "side"
    );

    // A signed-out account is never made global.
    std::fs::remove_file(w.paths.account_dir("personal").join(".credentials.json")).unwrap();
    let refused = account_fns::global(
        &w.ctx,
        account_fns::GlobalInput {
            account: "personal".into(),
            move_mode: account_fns::GlobalMove::IdleNowWorkingAfter,
        },
    )
    .await
    .unwrap_err();
    assert!(refused.contains("not signed in"), "{refused}");
}

#[tokio::test]
async fn before_tmux_3_two_claude_panes_in_a_window_stay_put() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("shared");
    let a = windows::create(&w.ctx, window("pair", claude("global", false)))
        .await
        .unwrap()
        .pane_id;
    w.claude(&a).await;
    assert!(w.claude(&a).await.movable, "alone in its window");
    // Through the worker's tmux and by its full path: the fake, never the
    // computer's own `claude` (tmux would give the pane this test's PATH).
    let fake = w.sandbox.dir.join("bin/claude");
    let split = std::process::Command::new(w.sandbox.dir.join("tmux"))
        .args([
            "-L",
            &w.sandbox.socket,
            "split-window",
            "-P",
            "-F",
            "#{pane_id}",
        ])
        .args(["-t", &a])
        .arg(fake.to_string_lossy().as_ref())
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .unwrap();
    let b = String::from_utf8_lossy(&split.stdout).trim().to_string();
    assert!(b.starts_with('%'), "{split:?}");
    w.claude(&b).await;
    assert!(w.screen(&b).contains("(demo)"), "the fake Claude Code");
    let old = w.tmux_out(&["-V"]).starts_with("tmux 2.");
    for pane in [&a, &b] {
        let info = w.claude(pane).await;
        if old {
            // One mark per window there: neither pane can keep its own account.
            assert!(!info.movable, "{info:?}");
            assert_eq!(
                info.not_movable_reason.as_deref(),
                Some(tmux::accounts::panes::SHARED_MARKS)
            );
        } else {
            assert!(info.movable, "{info:?}");
        }
    }
    if old {
        let refused = panes::account(
            &w.ctx,
            panes::PaneAccountInput {
                pane: a.clone(),
                account: "default".into(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(refused, tmux::accounts::panes::SHARED_MARKS);
    }

    // A switch that leaves panes where they run pins them there, except a
    // window mark two Claude Code panes share: that one is left alone.
    account_fns::add(
        &w.ctx,
        account_fns::AddAccountInput {
            name: "Personal".into(),
            color: "green".into(),
            shared: None,
        },
    )
    .await
    .unwrap();
    signed_in(&w.paths.account_dir("personal"));
    let switched = account_fns::global(
        &w.ctx,
        account_fns::GlobalInput {
            account: "personal".into(),
            move_mode: account_fns::GlobalMove::NewOnly,
        },
    )
    .await
    .unwrap();
    assert!(switched.stayed.contains(&a), "{switched:?}");
    let scope = if old { "-w" } else { "-p" };
    let mark = w.tmux_out(&["show-options", scope, "-v", "-t", &a, "@iii_claude_account"]);
    assert_eq!(mark, if old { "global" } else { "default" });
}

#[tokio::test]
async fn claude_slow_to_stop_comes_back_on_its_account() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("late");
    account_fns::add(
        &w.ctx,
        account_fns::AddAccountInput {
            name: "Personal".into(),
            color: "green".into(),
            shared: None,
        },
    )
    .await
    .unwrap();
    signed_in(&w.paths.account_dir("personal"));
    let a = windows::create(&w.ctx, window("late", claude("global", false)))
        .await
        .unwrap()
        .pane_id;
    let shell = windows::create(&w.ctx, window("shell", None))
        .await
        .unwrap()
        .pane_id;
    w.tmux(&["send-keys", "-t", &shell, "-l", "claude"]);
    w.tmux(&["send-keys", "-t", &shell, "Enter"]);
    let session = w.claude(&a).await.session_id.unwrap();
    w.claude(&shell).await;
    let pid = w.claude_pid(&a).await;
    let shell_pid = w.claude_pid(&shell).await;
    // Their SessionEnd hooks take fourteen seconds: more than a move waits.
    for claude_pid in [pid, shell_pid] {
        std::fs::write(
            w.sandbox.dir.join(format!("state/{claude_pid}.linger")),
            "14",
        )
        .unwrap();
    }
    let move_to_personal = |pane: String| {
        panes::account(
            &w.ctx,
            panes::PaneAccountInput {
                pane,
                account: "personal".into(),
            },
        )
    };
    let (respawned, typed) =
        tokio::join!(move_to_personal(a.clone()), move_to_personal(shell.clone()));
    assert_eq!(respawned.unwrap_err(), tmux::accounts::moves::DID_NOT_STOP);
    assert_eq!(
        typed.unwrap_err(),
        tmux::accounts::moves::DID_NOT_STOP_SHELL
    );
    let scope = if w.tmux_out(&["-V"]).starts_with("tmux 2.") {
        "-w"
    } else {
        "-p"
    };
    let remain = || w.tmux_out(&["show-options", scope, "-v", "-t", &a, "remain-on-exit"]);
    assert_eq!(remain(), "on", "kept while Claude Code still ends");
    // No other move (no second SIGTERM) while they still end.
    for pane in [&a, &shell] {
        let again = move_to_personal(pane.clone()).await.unwrap_err();
        assert_eq!(
            again,
            tmux::accounts::moves::BUSY,
            "{pane}: no other move meanwhile"
        );
    }
    // Once it ends, the pane stays and Claude Code starts again where it
    // was, resuming its conversation; remain-on-exit goes back as it was.
    let back = eventually("Claude Code back on its account", || async {
        let info = w.pane(&a).await?.claude?;
        let now = w.collector.claude_panes(true).await.get(&a)?.claude_pid?;
        (now != pid && info.session_id.is_some()).then_some(info)
    })
    .await;
    assert_eq!(back.account.as_deref(), Some("default"));
    assert_eq!(back.session_id.as_deref(), Some(session.as_str()));
    assert!(w.screen(&a).contains("Resumed on the default account"));
    eventually("remain-on-exit put back", || async {
        (remain() == "off").then_some(())
    })
    .await;
    // The shell has its terminal back; nothing was typed into it.
    eventually("the shell's Claude Code ended", || async {
        let gone = w
            .collector
            .claude_panes(true)
            .await
            .get(&shell)
            .is_none_or(|state| state.claude_pid != Some(shell_pid));
        gone.then_some(())
    })
    .await;
    assert!(!w.screen(&shell).contains("Resumed on"));
}

#[tokio::test]
async fn a_pane_pinned_during_a_switch_stays_where_it_was_pinned() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("pinned");
    for name in ["Personal", "Side"] {
        account_fns::add(
            &w.ctx,
            account_fns::AddAccountInput {
                name: name.into(),
                color: "green".into(),
                shared: Some(vec![]),
            },
        )
        .await
        .unwrap();
    }
    for id in ["personal", "side"] {
        signed_in(&w.paths.account_dir(id));
    }
    let a = windows::create(&w.ctx, window("a", claude("global", false)))
        .await
        .unwrap()
        .pane_id;
    w.claude(&a).await;
    let pid = w.claude_pid(&a).await;
    let scope = if w.tmux_out(&["-V"]).starts_with("tmux 2.") {
        "-w"
    } else {
        "-p"
    };
    let tmux_now = w.collector.tmux().await.unwrap();
    w.accounts()
        .load_accounts_now(vec!["personal".into()])
        .await;

    // The switch works from a list made before someone pinned the pane
    // where it runs: the pane is looked at again and left alone.
    let before = w.collector.claude_panes(true).await;
    w.tmux(&[
        "set-option",
        scope,
        "-t",
        &a,
        "@iii_claude_account",
        "default",
    ]);
    let switched = tmux::accounts::moves::switch_global(
        &tmux_now,
        w.accounts(),
        &before,
        "personal",
        tmux::accounts::moves::SwitchMode::IdleNowWorkingAfter,
        tmux::accounts::moves::SwitchBy::Person,
    )
    .await
    .unwrap();
    assert!(
        switched.moved.is_empty() && switched.after_turn.is_empty() && switched.stayed.is_empty(),
        "{switched:?}"
    );
    assert_eq!(w.claude_pid(&a).await, pid, "not moved");

    // move_idle too: pinned elsewhere since its list was made, it stays.
    let before = w.collector.claude_panes(true).await;
    w.tmux(&["set-option", scope, "-t", &a, "@iii_claude_account", "side"]);
    let moved =
        tmux::accounts::moves::move_idle(&tmux_now, w.accounts(), &before, "default", "personal")
            .await
            .unwrap();
    assert!(moved.is_empty(), "{moved:?}");
    assert_eq!(w.claude_pid(&a).await, pid, "not moved");
}

/// The Android lane's P15: a pane that went along with a switch after its
/// turn goes along with the next switch too. Each move used to drop its
/// account's sign-in from the cache, so the next pane of the same switch (or
/// of the same pending round, or of "move the idle ones") found that account
/// "not signed in".
#[tokio::test]
async fn panes_that_went_along_once_go_along_again() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("again");
    account_fns::add(
        &w.ctx,
        account_fns::AddAccountInput {
            name: "Personal".into(),
            color: "green".into(),
            shared: Some(vec![]),
        },
    )
    .await
    .unwrap();
    for dir in [w.paths.claude_home.clone(), w.paths.account_dir("personal")] {
        signed_in(&dir);
    }
    let mut both = Vec::new();
    for name in ["a", "b"] {
        let pane = windows::create(&w.ctx, window(name, claude("global", false)))
            .await
            .unwrap()
            .pane_id;
        w.claude(&pane).await;
        both.push(pane);
    }
    both.sort();
    let (a, b) = (both[0].clone(), both[1].clone());
    let global = |account: &str| {
        account_fns::global(
            &w.ctx,
            account_fns::GlobalInput {
                account: account.into(),
                move_mode: account_fns::GlobalMove::IdleNowWorkingAfter,
            },
        )
    };

    // b works: a goes now, b once its turn ends.
    w.set_state(&b, "working").await;
    let switched = global("personal").await.unwrap();
    assert_eq!(switched.moved, vec![a.clone()]);
    assert_eq!(switched.after_turn, vec![b.clone()]);
    w.set_state(&b, "idle").await;
    w.pending_tick().await;
    for pane in [&a, &b] {
        assert_eq!(w.settled_on(pane, "personal").await.follow, Follow::Global);
        // Restarted on the account the conversation was copied to, by name.
        let started = w.tmux_out(&["display", "-p", "-t", pane, "#{pane_start_command}"]);
        assert!(
            started.contains(&format!(
                "CLAUDE_CONFIG_DIR={}",
                w.paths.account_dir("personal").display()
            )),
            "{started}"
        );
    }
    // Someone switched: noted for the apps, with what moved.
    let last = account_fns::list(&w.ctx, EmptyInput {})
        .await
        .unwrap()
        .last_switch
        .expect("last switch");
    assert_eq!(
        (last.from.as_str(), last.to.as_str(), last.auto, last.reason),
        ("default", "personal", false, None)
    );
    assert_eq!(
        (last.moved.clone(), last.after_turn.clone()),
        (vec![a.clone()], vec![b.clone()])
    );

    // Back: both idle, both go, one after the other.
    let switched = global("default").await.unwrap();
    assert_eq!(switched.moved, both, "{switched:?}");
    assert!(switched.stayed.is_empty() && switched.after_turn.is_empty());
    for pane in [&a, &b] {
        let info = w.settled_on(pane, "default").await;
        assert_eq!(info.follow, Follow::Global);
        assert_eq!(info.move_error, None);
    }

    // Both waiting for their turns: one pending round moves both.
    for pane in [&a, &b] {
        w.set_state(pane, "working").await;
    }
    let switched = global("personal").await.unwrap();
    assert_eq!(switched.after_turn, both, "{switched:?}");
    for pane in [&a, &b] {
        w.set_state(pane, "idle").await;
    }
    w.pending_tick().await;
    for pane in [&a, &b] {
        let info = w.settled_on(pane, "personal").await;
        assert_eq!(info.move_error, None);
    }

    // "Move the idle ones": both go.
    let moved = account_fns::move_idle(
        &w.ctx,
        account_fns::MoveIdleInput {
            from: "personal".into(),
            to: "default".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(moved.moved, both);
    for pane in [&a, &b] {
        assert_eq!(w.settled_on(pane, "default").await.follow, Follow::Pinned);
    }
}
