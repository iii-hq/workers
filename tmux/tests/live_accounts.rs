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
use tmux::functions::{self, sessions, windows, Ctx, EmptyInput};
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
            "#!/bin/sh\nPATH='{}'\nexport PATH\nexec '{}' \"$@\"\n",
            path,
            real_tmux.display()
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

    // Pinned to an account, skipping permission prompts.
    let pinned = windows::create(&w.ctx, window("pinned", claude("personal", true)))
        .await
        .unwrap();
    assert_eq!(
        pinned.runs.as_deref(),
        Some(
            format!(
                "env CLAUDE_CONFIG_DIR={} claude --dangerously-skip-permissions",
                personal_dir.display()
            )
            .as_str()
        )
    );
    let info = w.claude(&pinned.pane_id).await;
    assert_eq!(info.account.as_deref(), Some("personal"));
    assert_eq!(info.follow, Follow::Pinned);
    assert!(info.skip_permissions);
    assert!(info.movable, "{info:?}");
    assert_eq!(
        w.tmux_out(&[
            "display",
            "-p",
            "-t",
            &pinned.pane_id,
            "#{@iii_claude_account}:#{@iii_skip_permissions}"
        ]),
        "personal:1"
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
