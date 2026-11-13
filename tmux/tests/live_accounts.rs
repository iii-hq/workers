//! Claude Code accounts against a real tmux on a private socket
//! (`-L iii-tmux-acct-<uuid>`) with the demo kit's fake Claude Code and a
//! temporary home: never the developer's sessions, environment or sign-in.
//! The fake reads its sign-in again on every line typed into it and notes
//! the account it used, so a switch of the global account (agent-accounts
//! G1-G12) shows in a session that was never restarted.
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
    // Its sign-in (`auth login`, `/login`) uses the demo's people.
    std::fs::copy(
        fake.with_file_name("setup_demo.py"),
        bin.join("setup_demo.py"),
    )
    .unwrap();
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
    // Sign-ins run the fake too, never the real Claude Code or a browser.
    accounts.logins.set_runner(tmux::accounts::login::Runner {
        claude: Some(bin.join("claude")),
        env: vec![
            (
                "FAKE_CLAUDE_SANDBOX".to_string(),
                dir.to_string_lossy().into_owned(),
            ),
            (
                "FAKE_CLAUDE_STATE_DIR".to_string(),
                dir.join("state").to_string_lossy().into_owned(),
            ),
        ],
        opener: Some(PathBuf::from("/bin/true")),
        ..Default::default()
    });
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

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn write_json(path: &Path, value: &serde_json::Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, value.to_string()).unwrap();
}

fn read_json(path: &Path) -> serde_json::Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// The profile Claude Code keeps for account `label` (`oauthAccount`).
fn profile(label: &str) -> serde_json::Value {
    serde_json::json!({"accountUuid": format!("uuid-{label}"),
        "emailAddress": format!("{label}@demo.dev"), "displayName": label})
}

/// A sign-in of account `label` in `dir` (its own chain: `demo-<label>`
/// for the fake Claude Code, a refresh token of its own) and its profile
/// in `json`, as `claude auth login` leaves them.
fn sign_in(dir: &Path, json: &Path, label: &str) {
    write_json(
        &dir.join(".credentials.json"),
        &serde_json::json!({"claudeAiOauth": {"accessToken": format!("demo-{label}"),
            "refreshToken": format!("rt-{label}-{}", uuid::Uuid::new_v4().simple()),
            "expiresAt": now_ms() + 3_600_000, "subscriptionType": "pro"}}),
    );
    let mut config = read_json(json);
    if !config.is_object() {
        config = serde_json::json!({});
    }
    config["oauthAccount"] = profile(label);
    config["hasCompletedOnboarding"] = true.into();
    write_json(json, &config);
}

/// [`sign_in`] with a cached usage reading (`cachedUsageUtilization`): the
/// week at `week` %, resetting at `resets_ms`.
fn sign_in_with_usage(dir: &Path, json: &Path, label: &str, week: f64, resets_ms: u64) {
    sign_in(dir, json, label);
    let mut config = read_json(json);
    config["cachedUsageUtilization"] = serde_json::json!({"fetchedAtMs": now_ms(),
        "utilization": {"seven_day": {"utilization": week, "resets_at": resets_ms}}});
    write_json(json, &config);
}

/// The label of the token in `dir`'s sign-in (`work` for `demo-work`).
fn token_in(dir: &Path) -> Option<String> {
    read_json(&dir.join(".credentials.json"))["claudeAiOauth"]["accessToken"]
        .as_str()
        .map(|token| token.trim_start_matches("demo-").to_string())
}

impl World {
    /// `~/.claude` signed in to Work, then accounts `names` added (sharing
    /// `shared`), each signed in on its folder.
    async fn with_accounts(&self, names: &[&str], shared: Option<Vec<String>>) {
        sign_in(&self.paths.claude_home, &self.paths.default_json, "work");
        for name in names {
            let id = account_fns::add(
                &self.ctx,
                account_fns::AddAccountInput {
                    name: (*name).into(),
                    color: "green".into(),
                    shared: shared.clone(),
                },
            )
            .await
            .unwrap()
            .account
            .id;
            sign_in(
                &self.paths.account_dir(&id),
                &self.paths.account_json(&id),
                &id,
            );
        }
    }

    async fn global(&self, account: &str) -> Result<account_fns::GlobalOutput, String> {
        account_fns::global(
            &self.ctx,
            account_fns::GlobalInput {
                account: account.into(),
            },
        )
        .await
    }

    /// Type `text` into the fake Claude Code in `pane`, a request: the
    /// account whose token it used, read afresh.
    async fn ask(&self, pane: &str, text: &str) -> String {
        let pid = self.claude_pid(pane).await;
        let log = self.sandbox.dir.join("state/requests.log");
        let mine = |log: &Path| -> Vec<String> {
            std::fs::read_to_string(log)
                .unwrap_or_default()
                .lines()
                .filter_map(|line| line.split_once(' '))
                .filter(|(at, _)| *at == pid.to_string())
                .map(|(_, label)| label.to_string())
                .collect()
        };
        let before = mine(&log).len();
        self.tmux(&["send-keys", "-t", pane, "-l", text]);
        self.tmux(&["send-keys", "-t", pane, "Enter"]);
        eventually(&format!("a request from {pane}"), || async {
            let now = mine(&log);
            (now.len() > before).then(|| now.last().cloned()).flatten()
        })
        .await
    }

    fn mark(&self, pane: &str) -> String {
        let scope = if self.tmux_out(&["-V"]).starts_with("tmux 2.") {
            "-w"
        } else {
            "-p"
        };
        self.tmux_out(&[
            "show-options",
            scope,
            "-v",
            "-t",
            pane,
            "@iii_claude_account",
        ])
    }

    fn start_command(&self, pane: &str) -> String {
        self.tmux_out(&["display", "-p", "-t", pane, "#{pane_start_command}"])
    }

    /// The pane's Claude Code on the slot once it is `account`'s, with no
    /// move waiting.
    async fn settled_on_slot(&self, pane: &str, account: &str) -> PaneClaude {
        eventually(&format!("{pane} on ~/.claude as {account}"), || async {
            self.pane(pane).await?.claude.filter(|claude| {
                claude.account.as_deref() == Some(account)
                    && claude.config_dir.as_deref() == Some("~/.claude")
                    && claude.session_id.is_some()
                    && claude.pending.is_none()
            })
        })
        .await
    }
}

fn wire_code(error: &str) -> serde_json::Value {
    serde_json::from_str(error).unwrap_or_else(|_| panic!("not a coded error: {error}"))
}

#[tokio::test]
async fn claude_starts_on_the_account_asked_for() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("create");
    w.with_accounts(&["Personal"], Some(vec!["conversations".into()]))
        .await;
    let personal_dir = w.paths.account_dir("personal");
    let default_dir = w.paths.account_dir("default");
    assert!(default_dir.is_dir(), "default has a folder (G2)");

    // Pinned to an account that isn't global: on its folder, skipping
    // permission prompts (refused as root, where Claude Code refuses it).
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
    assert_eq!(w.mark(&pinned.pane_id), "personal");
    // Its conversation went to the account's folder, which shares
    // `projects/` with ~/.claude.
    let session_id = info.session_id.clone().unwrap();
    let transcripts: Vec<_> = walk(&w.home().join(".claude/projects"))
        .into_iter()
        .filter(|path| path.ends_with(format!("{session_id}.jsonl")))
        .collect();
    assert_eq!(transcripts.len(), 1, "shared through the link");

    // Following the global account: plain `claude`, on ~/.claude, and
    // tmux's environment is left alone.
    let follows = windows::create(&w.ctx, window("follows", claude("global", false)))
        .await
        .unwrap();
    assert_eq!(follows.runs.as_deref(), Some("claude"));
    let info = w.claude(&follows.pane_id).await;
    assert_eq!(info.account.as_deref(), Some("default"));
    assert_eq!(info.config_dir.as_deref(), Some("~/.claude"));
    assert_eq!(info.follow, Follow::Global);
    assert!(!w
        .tmux(&["show-environment", "-g", "CLAUDE_CONFIG_DIR"])
        .status
        .success());

    // Pinned to the account that is global now: on ~/.claude too, kept
    // once it ends.
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
    assert_eq!(session.runs.as_deref(), Some("claude"));
    let info = w.claude(&session.pane_id).await;
    assert_eq!(info.account.as_deref(), Some("default"));
    assert_eq!(info.config_dir.as_deref(), Some("~/.claude"));
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

    // A session's own CLAUDE_CONFIG_DIR, or tmux's global one (set by
    // hand): a window on ~/.claude unsets it outright.
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
    w.tmux(&["set-environment", "-g", "CLAUDE_CONFIG_DIR", "/elsewhere"]);
    let named = windows::create(&w.ctx, window("global-env", claude("default", false)))
        .await
        .unwrap();
    assert_eq!(
        named.runs.as_deref(),
        Some("env -u CLAUDE_CONFIG_DIR claude")
    );
    w.tmux(&["set-environment", "-gu", "CLAUDE_CONFIG_DIR"]);

    // A folder is deleted only when nothing can still need it: never while
    // a pane is being moved. Default and the global account stay.
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
    assert!(account_fns::remove(
        &w.ctx,
        account_fns::RemoveAccountInput {
            account: "default".into(),
            delete_files: false,
        },
    )
    .await
    .is_err());

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

/// G1, G3: a switch moves the sign-ins; every Claude Code on ~/.claude
/// that isn't pinned (started by the worker or by hand) uses the new
/// account with its next request, with no restart.
#[tokio::test]
async fn followers_switch_with_their_next_message() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("follow");
    w.with_accounts(&["Personal"], None).await;
    let a = windows::create(&w.ctx, window("a", claude("global", false)))
        .await
        .unwrap()
        .pane_id;
    let shell = windows::create(&w.ctx, window("shell", None))
        .await
        .unwrap()
        .pane_id;
    w.tmux(&["send-keys", "-t", &shell, "-l", "claude"]);
    w.tmux(&["send-keys", "-t", &shell, "Enter"]);
    for pane in [&a, &shell] {
        let info = w.claude(pane).await;
        assert_eq!(
            info.follow,
            Follow::Global,
            "{pane}: on ~/.claude, not pinned"
        );
        assert_eq!(info.account.as_deref(), Some("default"));
    }
    assert_eq!(w.ask(&a, "one").await, "work");
    let pids = (w.claude_pid(&a).await, w.claude_pid(&shell).await);

    let switched = w.global("personal").await.unwrap();
    assert_eq!(switched.global, "personal");
    assert_eq!(switched.state, account_fns::GlobalState::Switched);
    assert_eq!(switched.following, 2);
    assert!(switched.restarted.is_empty() && switched.after_turn.is_empty());
    assert!(switched.waiting_for.is_empty());

    // The same processes, now as Personal.
    assert_eq!(w.ask(&a, "two").await, "personal");
    assert_eq!(w.ask(&shell, "three").await, "personal");
    assert_eq!(
        (w.claude_pid(&a).await, w.claude_pid(&shell).await),
        pids,
        "no restart"
    );
    for pane in [&a, &shell] {
        let info = w.settled_on_slot(pane, "personal").await;
        assert_eq!(info.follow, Follow::Global);
    }
    // One chain, one store (G2): Personal's in ~/.claude, Work's in
    // default's folder, none left in Personal's; the profiles swapped.
    assert_eq!(token_in(&w.paths.claude_home).as_deref(), Some("personal"));
    assert_eq!(
        token_in(&w.paths.account_dir("default")).as_deref(),
        Some("work")
    );
    assert_eq!(token_in(&w.paths.account_dir("personal")), None);
    assert_eq!(
        read_json(&w.paths.default_json)["oauthAccount"]["accountUuid"],
        "uuid-personal"
    );
    assert_eq!(
        read_json(&w.paths.account_json("default"))["oauthAccount"]["accountUuid"],
        "uuid-work"
    );
    assert!(!w.paths.root.join("swap.json").exists(), "journal gone");
    let listed = account_fns::list(&w.ctx, EmptyInput {}).await.unwrap();
    assert_eq!(listed.global, "personal");
    let last = listed.last_switch.unwrap();
    assert_eq!(
        (
            last.from.as_str(),
            last.to.as_str(),
            last.auto,
            last.following
        ),
        ("default", "personal", false, 2)
    );
    let personal = listed.accounts.iter().find(|a| a.id == "personal").unwrap();
    assert!(personal.signed_in);
    assert_eq!(personal.panes, 2, "the global account counts its followers");
    assert!(
        listed
            .accounts
            .iter()
            .find(|a| a.id == "default")
            .unwrap()
            .signed_in
    );
    // tmux's environment was never touched.
    assert!(!w
        .tmux(&["show-environment", "-g", "CLAUDE_CONFIG_DIR"])
        .status
        .success());

    // The same account again: nothing moves.
    let again = w.global("personal").await.unwrap();
    assert_eq!(again.state, account_fns::GlobalState::Switched);
    assert_eq!(w.ask(&a, "four").await, "personal");
    // And back.
    w.global("default").await.unwrap();
    assert_eq!(w.ask(&a, "five").await, "work");
    assert_eq!(
        token_in(&w.paths.account_dir("personal")).as_deref(),
        Some("personal")
    );

    // A signed-out account is never made global.
    std::fs::remove_file(w.paths.account_dir("personal").join(".credentials.json")).unwrap();
    let refused = w.global("personal").await.unwrap_err();
    assert_eq!(wire_code(&refused)["code"], "not_signed_in", "{refused}");
    assert_eq!(w.accounts().registry().global, "default");
}

/// G4: windows pinned to the new account restart on ~/.claude; those
/// pinned to the old one restart on its folder, after their turn when
/// they are in one.
#[tokio::test]
async fn pinned_windows_restart_where_their_account_is() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("pinned");
    w.with_accounts(&["Personal", "Side"], Some(vec![])).await;
    let create = |name: &'static str, account: &'static str| {
        let ctx = &w.ctx;
        async move {
            windows::create(ctx, window(name, claude(account, false)))
                .await
                .unwrap()
                .pane_id
        }
    };
    let p = create("p", "personal").await;
    let q = create("q", "default").await;
    let r = create("r", "default").await;
    let s = create("s", "side").await;
    let f = create("f", "global").await;
    let mut sessions = HashMap::new();
    for pane in [&p, &q, &r, &s, &f] {
        sessions.insert(pane.clone(), w.claude(pane).await.session_id.unwrap());
    }
    w.set_state(&r, "working").await;
    let side_pid = w.claude_pid(&s).await;
    let f_pid = w.claude_pid(&f).await;

    let switched = w.global("personal").await.unwrap();
    let mut restarted = switched.restarted.clone();
    restarted.sort();
    let mut expected = vec![p.clone(), q.clone()];
    expected.sort();
    assert_eq!(restarted, expected, "{switched:?}");
    assert_eq!(switched.after_turn, vec![r.clone()]);
    assert_eq!(switched.following, 1);

    // p: on ~/.claude now, still pinned to Personal, its conversation resumed.
    let info = w.settled_on_slot(&p, "personal").await;
    assert_eq!(info.follow, Follow::Pinned);
    assert_eq!(info.session_id.as_ref(), sessions.get(&p));
    assert_eq!(w.mark(&p), "personal");
    assert!(
        w.start_command(&p)
            .contains("env -u CLAUDE_CONFIG_DIR claude --resume"),
        "{}",
        w.start_command(&p)
    );
    assert_eq!(w.ask(&p, "hello").await, "personal");
    // q: on default's folder, as Work.
    let info = w.settled_on(&q, "default").await;
    assert_eq!(info.follow, Follow::Pinned);
    assert_eq!(info.session_id.as_ref(), sessions.get(&q));
    assert!(w.start_command(&q).contains(&format!(
        "CLAUDE_CONFIG_DIR={}",
        w.paths.account_dir("default").display()
    )));
    assert_eq!(w.ask(&q, "hello").await, "work");
    // r: finishes its turn on ~/.claude (as Personal now), then goes.
    let info = w.pane(&r).await.unwrap().claude.unwrap();
    assert_eq!(info.pending.as_deref(), Some("default"));
    assert_eq!(w.ask(&r, "mid-turn").await, "personal");
    w.set_state(&r, "idle").await;
    w.pending_tick().await;
    let info = w.settled_on(&r, "default").await;
    assert_eq!(info.session_id.as_ref(), sessions.get(&r));
    assert_eq!(w.ask(&r, "after").await, "work");
    // s (another account) and f (a follower) were not restarted.
    assert_eq!(w.claude_pid(&s).await, side_pid);
    assert_eq!(w.claude_pid(&f).await, f_pid);
    assert_eq!(w.ask(&f, "hello").await, "personal");
    let last = account_fns::list(&w.ctx, EmptyInput {})
        .await
        .unwrap()
        .last_switch
        .unwrap();
    assert_eq!(last.after_turn, vec![r.clone()]);
    assert_eq!(last.following, 1);
}

/// G4: a switch waits for the windows on the new account's folder that are
/// in a turn; the worker finishes it once they are idle. Switching to the
/// global account calls it off.
#[tokio::test]
async fn a_switch_waits_for_a_pinned_window_in_a_turn() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("wait");
    w.with_accounts(&["Personal"], None).await;
    let p = windows::create(&w.ctx, window("p", claude("personal", false)))
        .await
        .unwrap()
        .pane_id;
    let f = windows::create(&w.ctx, window("f", claude("global", false)))
        .await
        .unwrap()
        .pane_id;
    w.claude(&p).await;
    w.claude(&f).await;
    w.set_state(&p, "working").await;

    let waiting = w.global("personal").await.unwrap();
    assert_eq!(waiting.state, account_fns::GlobalState::Waiting);
    assert_eq!(waiting.global, "default");
    assert_eq!(waiting.waiting_for, vec![p.clone()]);
    assert_eq!(waiting.following, 1);
    let listed = account_fns::list(&w.ctx, EmptyInput {}).await.unwrap();
    let pending = listed.pending_global.expect("pending_global");
    assert_eq!(
        (pending.account.as_str(), pending.by.as_str()),
        ("personal", "person")
    );
    assert_eq!(pending.waiting_for, vec![p.clone()]);
    assert_eq!(w.ask(&f, "still").await, "work");
    // The account a switch waits for can't be removed.
    assert!(account_fns::remove(
        &w.ctx,
        account_fns::RemoveAccountInput {
            account: "personal".into(),
            delete_files: false,
        },
    )
    .await
    .is_err());
    // Still in its turn: a round changes nothing.
    w.pending_tick().await;
    assert_eq!(w.accounts().registry().global, "default");

    // Switching to the global account calls it off.
    let cancelled = w.global("default").await.unwrap();
    assert_eq!(cancelled.state, account_fns::GlobalState::Switched);
    assert!(w.accounts().registry().pending_global.is_none());

    // Again, and this time the turn ends: the worker finishes it.
    w.global("personal").await.unwrap();
    let p_pid = w.claude_pid(&p).await;
    w.set_state(&p, "idle").await;
    w.pending_tick().await;
    let registry = w.accounts().registry();
    assert_eq!(registry.global, "personal");
    assert!(registry.pending_global.is_none());
    let last = registry.last_switch.unwrap();
    assert_eq!(last.restarted, vec![p.clone()]);
    assert!(!last.auto);
    let info = w.settled_on_slot(&p, "personal").await;
    assert_eq!(info.follow, Follow::Pinned);
    assert_ne!(w.claude_pid(&p).await, p_pid, "restarted on ~/.claude");
    assert_eq!(w.ask(&f, "now").await, "personal");
    assert_eq!(w.ask(&p, "now").await, "personal");
}

/// G4: a Claude Code on the new account's folder that the worker can't
/// restart refuses the switch, naming it.
#[tokio::test]
async fn a_claude_that_cant_restart_blocks_the_switch() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("blocked");
    w.with_accounts(&["Personal"], None).await;
    // Claude Code under another program: nothing to respawn, no shell to
    // type into.
    let fake = w.sandbox.dir.join("bin/claude");
    let nested = windows::create(
        &w.ctx,
        windows::CreateWindowInput {
            command: Some(format!(
                "env CLAUDE_CONFIG_DIR={} python3 -c 'import subprocess,sys; sys.exit(subprocess.call([\"{}\"]))'",
                w.paths.account_dir("personal").display(),
                fake.display()
            )),
            ..window("nested", None)
        },
    )
    .await
    .unwrap()
    .pane_id;
    let info = w.claude(&nested).await;
    assert_eq!(info.account.as_deref(), Some("personal"));
    assert_eq!(info.follow, Follow::Manual);
    assert!(!info.movable);
    let refused = w.global("personal").await.unwrap_err();
    let wire = wire_code(&refused);
    assert_eq!(wire["code"], "blocked", "{refused}");
    assert_eq!(wire["blocked_by"], serde_json::json!([nested.clone()]));
    assert!(wire["message"].as_str().unwrap().contains(&nested));
    assert_eq!(w.accounts().registry().global, "default");
    assert_eq!(token_in(&w.paths.claude_home).as_deref(), Some("work"));
    assert_eq!(
        token_in(&w.paths.account_dir("personal")).as_deref(),
        Some("personal")
    );
}

/// G6, G10: a sign-in by hand on ~/.claude (`/login`) makes that account
/// global, adding it when nobody knows it; the global account signs in on
/// ~/.claude.
#[tokio::test]
async fn a_sign_in_by_hand_on_the_slot_makes_it_global() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("login");
    sign_in(&w.paths.claude_home, &w.paths.default_json, "work");
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
    // Personal signs in on its folder through the worker's hidden sign-in
    // (claude-login-ui: no tmux window; the fake takes `demo-ok`).
    let windows_before = w.tmux_out(&["list-windows", "-a", "-F", "#{window_id}"]);
    let login = account_fns::login_start(
        &w.ctx,
        account_fns::LoginStartInput {
            account: "personal".into(),
            browser: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        w.tmux_out(&["list-windows", "-a", "-F", "#{window_id}"]),
        windows_before,
        "no login window"
    );
    let done = account_fns::login_code(
        &w.ctx,
        account_fns::LoginCodeInput {
            login_id: login.login_id,
            code: "demo-ok#demo-state".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(done.account.id, "personal");
    assert_eq!(
        token_in(&w.paths.account_dir("personal")).as_deref(),
        Some("personal")
    );
    // The global account's sign-in runs on ~/.claude; cancelled here.
    let global_login = account_fns::login_start(
        &w.ctx,
        account_fns::LoginStartInput {
            account: "default".into(),
            browser: None,
        },
    )
    .await
    .unwrap();
    // While it runs, the global account doesn't change hands.
    let refused: serde_json::Value =
        serde_json::from_str(&w.global("personal").await.unwrap_err()).unwrap();
    assert_eq!(refused["code"], "signing_in", "{refused}");
    assert_eq!(w.accounts().registry().global, "default");
    account_fns::login_cancel(
        &w.ctx,
        account_fns::LoginCancelInput {
            login_id: global_login.login_id,
        },
    )
    .await
    .unwrap();

    let a = windows::create(&w.ctx, window("a", claude("global", false)))
        .await
        .unwrap()
        .pane_id;
    w.claude(&a).await;
    // Pinned to Work, the global account: on ~/.claude.
    let d = windows::create(&w.ctx, window("d", claude("default", false)))
        .await
        .unwrap()
        .pane_id;
    w.claude(&d).await;
    w.pending_tick().await;
    assert_eq!(w.accounts().registry().global, "default");

    // `/login` as Personal in a session on ~/.claude.
    w.tmux(&["send-keys", "-t", &a, "-l", "/login personal"]);
    w.tmux(&["send-keys", "-t", &a, "Enter"]);
    eventually("Personal global", || async {
        w.pending_tick().await;
        (w.accounts().registry().global == "personal").then_some(())
    })
    .await;
    let last = w.accounts().registry().last_switch.unwrap();
    assert_eq!(
        (
            last.from.as_str(),
            last.to.as_str(),
            last.reason.as_deref(),
            last.auto
        ),
        ("default", "personal", Some("login"), false)
    );
    assert_eq!(w.ask(&a, "who").await, "personal");
    // d runs as Personal on ~/.claude, and Work is signed out (its sign-in
    // was there): it follows the global account now, counted with a.
    let info = eventually("d follows", || async {
        w.pane(&d)
            .await?
            .claude
            .filter(|claude| claude.follow == Follow::Global)
    })
    .await;
    assert_eq!(info.account.as_deref(), Some("personal"));
    assert_eq!(w.mark(&d), "global");
    let last = w.accounts().registry().last_switch.unwrap();
    assert!(last.following >= 2, "{last:?}");
    assert_eq!(w.ask(&d, "who").await, "personal");
    // Work's sign-in was in ~/.claude: /login replaced it.
    let listed = account_fns::list(&w.ctx, EmptyInput {}).await.unwrap();
    assert!(
        !listed
            .accounts
            .iter()
            .find(|x| x.id == "default")
            .unwrap()
            .signed_in
    );

    // `/login` as someone nobody knows: added, all shared, global.
    w.tmux(&["send-keys", "-t", &a, "-l", "/login as:ana"]);
    w.tmux(&["send-keys", "-t", &a, "Enter"]);
    eventually("Ana global", || async {
        w.pending_tick().await;
        (w.accounts().registry().global == "ana").then_some(())
    })
    .await;
    let registry = w.accounts().registry();
    let ana = registry.get("ana").expect("Ana added");
    assert_eq!(ana.name, "Ana");
    assert_eq!(ana.shared.len(), 4);
    assert_eq!(
        registry.last_switch.as_ref().unwrap().reason.as_deref(),
        Some("login_new")
    );
    assert!(w.paths.account_dir("ana").is_dir());
    assert_eq!(
        token_in(&w.paths.account_dir("ana")),
        None,
        "its sign-in stays in ~/.claude"
    );
    assert_eq!(w.ask(&a, "who").await, "ana");
    // Switching away moves Ana's sign-in to her folder; Personal's older
    // sign-in, still in its folder, goes to ~/.claude.
    w.global("personal").await.unwrap();
    assert_eq!(
        token_in(&w.paths.account_dir("ana")).as_deref(),
        Some("ana")
    );
    assert_eq!(token_in(&w.paths.account_dir("personal")), None);
    assert_eq!(w.ask(&a, "who").await, "personal");

    // `/login` back to Ana within the minute after that switch: a sign-in
    // by hand, not Ana's profile written back by a late session. Ana is
    // global again with her profile on ~/.claude, and Personal, whose only
    // sign-in was there, reads signed out.
    w.tmux(&["send-keys", "-t", &a, "-l", "/login as:ana"]);
    w.tmux(&["send-keys", "-t", &a, "Enter"]);
    eventually("Ana global again", || async {
        w.pending_tick().await;
        (w.accounts().registry().global == "ana").then_some(())
    })
    .await;
    assert_eq!(
        w.accounts()
            .registry()
            .last_switch
            .unwrap()
            .reason
            .as_deref(),
        Some("login")
    );
    for _ in 0..3 {
        w.pending_tick().await;
    }
    assert_eq!(w.accounts().registry().global, "ana");
    assert_eq!(
        read_json(&w.paths.default_json)["oauthAccount"]["accountUuid"],
        read_json(&w.paths.account_json("ana"))["oauthAccount"]["accountUuid"],
        "~/.claude keeps Ana's profile"
    );
    assert_eq!(w.ask(&a, "who").await, "ana");
    let listed = account_fns::list(&w.ctx, EmptyInput {}).await.unwrap();
    assert!(
        !listed
            .accounts
            .iter()
            .find(|x| x.id == "personal")
            .unwrap()
            .signed_in
    );
}

/// G11: a registry and tmux of the build that kept the global account in
/// tmux's global environment.
#[tokio::test]
async fn an_upgrade_from_the_tmux_environment_build() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("upgrade");
    sign_in(&w.paths.claude_home, &w.paths.default_json, "work");
    let personal_dir = w.paths.account_dir("personal");
    sign_in(&personal_dir, &w.paths.account_json("personal"), "personal");
    write_json(
        &w.paths.registry_file(),
        &serde_json::json!({"version": 1, "global": "personal", "revision": 4,
            "accounts": [{"id": "default", "name": "Work", "color": "blue"},
                {"id": "personal", "name": "Personal", "color": "green",
                 "shared": ["settings", "extensions", "plugins", "conversations"], "created_at_ms": 1}],
            "rules": {"mode": "suggest", "at": "week_90", "five_hour": false, "move_idle": true,
                "back_after_reset": false, "order": []},
            "dismissed": [], "last_switch": null}),
    );
    assert!(w.accounts().managed());
    // 0.3.0 put Personal's folder in tmux's global environment, and a
    // window that followed the global account runs there.
    let personal_text = personal_dir.to_string_lossy().into_owned();
    w.tmux(&["set-environment", "-g", "CLAUDE_CONFIG_DIR", &personal_text]);
    let old = windows::create(
        &w.ctx,
        windows::CreateWindowInput {
            command: Some(format!("env CLAUDE_CONFIG_DIR={personal_text} claude")),
            ..window("old", None)
        },
    )
    .await
    .unwrap()
    .pane_id;
    let scope = if w.tmux_out(&["-V"]).starts_with("tmux 2.") {
        "-w"
    } else {
        "-p"
    };
    w.tmux(&[
        "set-option",
        scope,
        "-t",
        &old,
        "@iii_claude_account",
        "global",
    ]);
    w.claude(&old).await;
    let pid = w.claude_pid(&old).await;

    tmux::accounts::daemon::start(&w.collector, w.accounts()).await;

    assert!(!w
        .tmux(&["show-environment", "-g", "CLAUDE_CONFIG_DIR"])
        .status
        .success());
    let registry = w.accounts().registry();
    assert_eq!(registry.slot_version, 2);
    assert_eq!(registry.global, "default", "the account in ~/.claude");
    let last = registry.last_switch.clone().unwrap();
    assert_eq!(
        (last.from.as_str(), last.reason.as_deref(), last.auto),
        ("personal", Some("upgrade"), false)
    );
    assert_eq!(registry.get("default").unwrap().shared.len(), 4);
    assert!(w.paths.account_dir("default").is_dir());
    assert_eq!(
        read_json(&w.paths.account_json("default"))["oauthAccount"]["accountUuid"],
        "uuid-work"
    );
    let info = w.settled_on(&old, "personal").await;
    assert_eq!(info.follow, Follow::Pinned, "pinned where it ran");
    assert_eq!(w.mark(&old), "personal");
    assert_eq!(w.claude_pid(&old).await, pid, "not restarted");
    // New windows follow ~/.claude.
    let new = windows::create(&w.ctx, window("new", claude("global", false)))
        .await
        .unwrap();
    assert_eq!(new.runs.as_deref(), Some("claude"));
    assert_eq!(
        w.claude(&new.pane_id).await.account.as_deref(),
        Some("default")
    );
    // Once is enough: a second start changes nothing.
    let revision = w.accounts().registry().revision;
    tmux::accounts::daemon::start(&w.collector, w.accounts()).await;
    assert_eq!(w.accounts().registry().revision, revision);
}

#[tokio::test]
async fn panes_move_between_accounts_between_turns() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("move");
    // Nothing shared: a move has to copy the conversation.
    w.with_accounts(&["Personal", "Side"], Some(vec![])).await;
    let personal_dir = w.paths.account_dir("personal");

    // An idle pane that follows the global account moves at once to an
    // account's folder, and resumes its conversation there.
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
    assert!(w.screen(&a).contains("Resumed, signed in as personal"));
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

    // Working: the move back to ~/.claude waits for the end of the turn.
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
    let info = w.settled_on_slot(&a, "default").await;
    assert_eq!(info.follow, Follow::Global);
    assert!(w
        .start_command(&a)
        .contains("env -u CLAUDE_CONFIG_DIR claude"));

    // Cancelling a waiting move: back to what it is.
    w.set_state(&a, "waiting").await;
    assert_eq!(
        pane_account(&w, &a, "side").await.state,
        panes::PaneAccountState::AfterTurn
    );
    assert_eq!(
        pane_account(&w, &a, "global").await.state,
        panes::PaneAccountState::Cancelled
    );
    // Pinned to the global account where it runs: no restart.
    let pid = w.claude_pid(&a).await;
    assert_eq!(
        pane_account(&w, &a, "default").await.state,
        panes::PaneAccountState::Pinned
    );
    assert_eq!(w.mark(&a), "default");
    assert_eq!(
        pane_account(&w, &a, "global").await.state,
        panes::PaneAccountState::Following
    );
    w.set_state(&a, "idle").await;
    assert_eq!(w.claude_pid(&a).await, pid);

    // The palette's "move the idle ones": b is working, so only a goes.
    let b = windows::create(&w.ctx, window("b", claude("global", false)))
        .await
        .unwrap()
        .pane_id;
    w.claude(&b).await;
    w.set_state(&b, "working").await;
    let moved = account_fns::move_idle(
        &w.ctx,
        account_fns::MoveIdleInput {
            from: "default".into(),
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
        Some("default")
    );
}

#[tokio::test]
async fn claude_started_by_hand_moves_through_its_shell() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("shell");
    w.with_accounts(&["Personal"], None).await;
    let shell = windows::create(&w.ctx, window("shell", None))
        .await
        .unwrap()
        .pane_id;
    w.tmux(&["send-keys", "-t", &shell, "-l", "claude"]);
    w.tmux(&["send-keys", "-t", &shell, "Enter"]);
    let info = w.claude(&shell).await;
    // Plain `claude` by hand runs on ~/.claude: it follows (G12).
    assert_eq!(info.follow, Follow::Global);
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
        screen.contains("Resumed, signed in as personal"),
        "{screen}"
    );
    let pane_pid = w.tmux_out(&["display", "-p", "-t", &shell, "#{pane_pid}"]);
    let claude_pid = w.claude_pid(&shell).await;
    assert_ne!(pane_pid, claude_pid.to_string());

    // Started by hand on an account's folder: `manual`.
    let other = windows::create(&w.ctx, window("other", None))
        .await
        .unwrap()
        .pane_id;
    let typed = format!(
        "env CLAUDE_CONFIG_DIR={} claude",
        w.paths.account_dir("personal").display()
    );
    w.tmux(&["send-keys", "-t", &other, "-l", &typed]);
    w.tmux(&["send-keys", "-t", &other, "Enter"]);
    let info = w.claude(&other).await;
    assert_eq!(info.follow, Follow::Manual);
    assert_eq!(info.account.as_deref(), Some("personal"));
}

#[tokio::test]
async fn rules_suggest_switch_and_switch_back() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("rules");
    // Room for a slow machine to get through the steps before the reset.
    let reset_at = now_ms() + 12_000;
    sign_in_with_usage(
        &w.paths.claude_home,
        &w.paths.default_json,
        "work",
        95.0,
        reset_at,
    );
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
    sign_in_with_usage(
        &w.paths.account_dir("personal"),
        &w.paths.account_json("personal"),
        "personal",
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

    // Switch by itself, never to an account with Claude Code working on
    // its folder.
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
    let p = windows::create(&w.ctx, window("p", claude("personal", false)))
        .await
        .unwrap()
        .pane_id;
    w.claude(&p).await;
    w.set_state(&p, "working").await;
    tmux::accounts::daemon::run_rules(&w.collector).await;
    assert_eq!(
        w.accounts().registry().global,
        "default",
        "Personal is busy"
    );
    assert!(
        w.accounts().registry().pending_global.is_none(),
        "rules never wait"
    );
    w.set_state(&p, "idle").await;
    assert!(tmux::accounts::daemon::run_rules(&w.collector).await);
    let listed = account_fns::list(&w.ctx, EmptyInput {}).await.unwrap();
    assert_eq!(listed.global, "personal");
    let last = listed.last_switch.expect("last switch");
    assert!(last.auto);
    assert_eq!(
        (last.from.as_str(), last.to.as_str(), last.reason.as_deref()),
        ("default", "personal", Some("week_90"))
    );
    assert_eq!(last.following, 1);
    assert_eq!(last.restarted, vec![p.clone()]);
    assert_eq!(last.back_at_ms, Some(reset_at));
    assert_eq!(w.ask(&a, "now").await, "personal");
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
    assert_eq!(w.ask(&a, "back").await, "work");
}

#[tokio::test]
async fn a_person_switching_calls_off_the_switch_back() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("back");
    let reset_at = now_ms() + 8_000;
    sign_in_with_usage(
        &w.paths.claude_home,
        &w.paths.default_json,
        "work",
        95.0,
        reset_at,
    );
    // Signed in only once both exist: nothing has read their sign-ins yet.
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
        sign_in_with_usage(
            &w.paths.account_dir(id),
            &w.paths.account_json(id),
            id,
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
    w.global("side").await.unwrap();
    let listed = account_fns::list(&w.ctx, EmptyInput {}).await.unwrap();
    assert_eq!(listed.last_switch.unwrap().back_at_ms, None);

    // The rules decided on a registry written since, even back to the same
    // values (side → personal → side): they switch nothing.
    let (decided_on, generation) = w.accounts().registry_at();
    for account in ["personal", "side"] {
        w.global(account).await.unwrap();
    }
    assert_eq!(w.accounts().registry().global, decided_on.global);
    let tmux_now = w.collector.tmux().await.unwrap();
    let panes_now = w.collector.claude_panes(true).await;
    let stale = tmux::accounts::switch::make_global(
        &tmux_now,
        w.accounts(),
        &panes_now,
        "personal",
        tmux::accounts::switch::By::Rules {
            generation,
            record: tmux::model::AccountSwitch {
                from: "side".into(),
                to: "personal".into(),
                reason: Some("week_90".into()),
                auto: true,
                following: 0,
                restarted: Vec::new(),
                after_turn: Vec::new(),
                at_ms: now_ms(),
                back_at_ms: None,
            },
        },
    )
    .await
    .unwrap_err();
    assert_eq!(stale, tmux::accounts::switch::SwitchError::Stale);
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
}

#[tokio::test]
async fn before_tmux_3_two_claude_panes_in_a_window_stay_put() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("shared");
    w.with_accounts(&["Personal"], None).await;
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

    // On ~/.claude they need no mark of their own: both follow a switch.
    let switched = w.global("personal").await.unwrap();
    assert_eq!(switched.following, 2, "{switched:?}");
    assert!(switched.restarted.is_empty());
    for pane in [&a, &b] {
        assert_eq!(w.ask(pane, "hi").await, "personal");
    }
}

#[tokio::test]
async fn claude_slow_to_stop_comes_back_on_its_account() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("late");
    w.with_accounts(&["Personal"], None).await;
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
    // One move at a time (the swap gate): the pane the worker started
    // first, then the shell.
    let respawned = move_to_personal(a.clone()).await;
    assert_eq!(respawned.unwrap_err(), tmux::accounts::moves::DID_NOT_STOP);
    let scope = if w.tmux_out(&["-V"]).starts_with("tmux 2.") {
        "-w"
    } else {
        "-p"
    };
    let remain = || w.tmux_out(&["show-options", scope, "-v", "-t", &a, "remain-on-exit"]);
    assert_eq!(remain(), "on", "kept while Claude Code still ends");
    // No other move (no second SIGTERM) while it still ends.
    assert_eq!(
        move_to_personal(a.clone()).await.unwrap_err(),
        tmux::accounts::moves::BUSY,
        "no other move meanwhile"
    );
    // Once it ends, the pane stays and Claude Code starts again where it
    // was, resuming its conversation; remain-on-exit goes back as it was.
    let back = eventually("Claude Code back where it was", || async {
        let info = w.pane(&a).await?.claude?;
        let now = w.collector.claude_panes(true).await.get(&a)?.claude_pid?;
        (now != pid && info.session_id.is_some()).then_some(info)
    })
    .await;
    assert_eq!(back.account.as_deref(), Some("default"));
    assert_eq!(back.config_dir.as_deref(), Some("~/.claude"));
    assert_eq!(back.session_id.as_deref(), Some(session.as_str()));
    assert!(w.screen(&a).contains("Resumed, signed in as work"));
    eventually("remain-on-exit put back", || async {
        (remain() == "off").then_some(())
    })
    .await;

    let typed = move_to_personal(shell.clone()).await;
    assert_eq!(
        typed.unwrap_err(),
        tmux::accounts::moves::DID_NOT_STOP_SHELL
    );
    assert_eq!(
        move_to_personal(shell.clone()).await.unwrap_err(),
        tmux::accounts::moves::BUSY,
        "no other move meanwhile"
    );
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
    assert!(!w.screen(&shell).contains("Resumed"));
}

#[tokio::test]
async fn a_pane_pinned_meanwhile_is_not_moved_by_move_idle() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("pinned-meanwhile");
    w.with_accounts(&["Personal", "Side"], Some(vec![])).await;
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
    // Pinned elsewhere since its list was made: it stays.
    let before = w.collector.claude_panes(true).await;
    w.tmux(&["set-option", scope, "-t", &a, "@iii_claude_account", "side"]);
    let moved =
        tmux::accounts::moves::move_idle(&tmux_now, w.accounts(), &before, "default", "personal")
            .await
            .unwrap();
    assert!(moved.is_empty(), "{moved:?}");
    assert_eq!(w.claude_pid(&a).await, pid, "not moved");
}

/// The Android lane's P15: a pane that went along once goes along again.
/// Each move used to drop its account's sign-in from the cache, so the next
/// pane of the same switch (or of the same pending round, or of "move the
/// idle ones") found that account "not signed in".
#[tokio::test]
async fn panes_that_went_along_once_go_along_again() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("again");
    w.with_accounts(&["Personal"], Some(vec![])).await;
    let mut both = Vec::new();
    for name in ["a", "b"] {
        // Pinned to Work, which is global: on ~/.claude.
        let pane = windows::create(&w.ctx, window(name, claude("default", false)))
            .await
            .unwrap()
            .pane_id;
        w.claude(&pane).await;
        both.push(pane);
    }
    both.sort();

    // Both working: they go to Work's folder once their turns end, one
    // pending round for both.
    for pane in &both {
        w.set_state(pane, "working").await;
    }
    let switched = w.global("personal").await.unwrap();
    assert_eq!(switched.after_turn, both, "{switched:?}");
    for pane in &both {
        w.set_state(pane, "idle").await;
    }
    w.pending_tick().await;
    for pane in &both {
        let info = w.settled_on(pane, "default").await;
        assert_eq!(info.move_error, None);
        assert_eq!(info.follow, Follow::Pinned);
        let started = w.start_command(pane);
        assert!(
            started.contains(&format!(
                "CLAUDE_CONFIG_DIR={}",
                w.paths.account_dir("default").display()
            )),
            "{started}"
        );
    }

    // Back: both idle, both restart on ~/.claude, one after the other.
    let switched = w.global("default").await.unwrap();
    let mut restarted = switched.restarted.clone();
    restarted.sort();
    assert_eq!(restarted, both, "{switched:?}");
    for pane in &both {
        let info = w.settled_on_slot(pane, "default").await;
        assert_eq!(info.move_error, None);
    }

    // "Move the idle ones" to Personal's folder: both go.
    let moved = account_fns::move_idle(
        &w.ctx,
        account_fns::MoveIdleInput {
            from: "default".into(),
            to: "personal".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(moved.moved, both);
    for pane in &both {
        assert_eq!(w.settled_on(pane, "personal").await.follow, Follow::Pinned);
    }
}

/// A pane pinned to the global account that can't be moved (Claude Code
/// inside another program) stays on ~/.claude through a switch, so it runs
/// as the new account: it follows that account from then on, and says why.
#[tokio::test]
async fn a_pinned_pane_that_cant_move_follows_the_new_account() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("unmovable");
    w.with_accounts(&["Personal"], None).await;
    let fake = w.sandbox.dir.join("bin/claude");
    let nested = windows::create(
        &w.ctx,
        windows::CreateWindowInput {
            command: Some(format!(
                "python3 -c 'import subprocess,sys; sys.exit(subprocess.call([\"{}\"]))'",
                fake.display()
            )),
            ..window("nested", None)
        },
    )
    .await
    .unwrap()
    .pane_id;
    let info = w.claude(&nested).await;
    assert_eq!(info.account.as_deref(), Some("default"));
    assert!(!info.movable, "{info:?}");
    let scope = if w.tmux_out(&["-V"]).starts_with("tmux 2.") {
        "-w"
    } else {
        "-p"
    };
    w.tmux(&[
        "set-option",
        scope,
        "-t",
        &nested,
        "@iii_claude_account",
        "default",
    ]);
    eventually("pinned to Work", || async {
        w.pane(&nested)
            .await?
            .claude
            .filter(|claude| claude.follow == Follow::Pinned)
    })
    .await;

    let switched = w.global("personal").await.unwrap();
    assert_eq!(switched.following, 1, "{switched:?}");
    assert!(switched.restarted.is_empty(), "{switched:?}");
    let info = eventually("it follows Personal", || async {
        w.pane(&nested)
            .await?
            .claude
            .filter(|claude| claude.follow == Follow::Global)
    })
    .await;
    assert_eq!(info.account.as_deref(), Some("personal"));
    let why = info.move_error.expect("it says why");
    assert!(
        why.contains("no longer global") && why.contains("follows the global account (Personal)"),
        "{why}"
    );
    assert_eq!(w.mark(&nested), "global");
    assert_eq!(w.ask(&nested, "who").await, "personal");
    let last = w.accounts().registry().last_switch.unwrap();
    assert_eq!(last.following, 1);
}

/// A pane pinned to the global account whose Claude Code takes longer than
/// ten seconds to stop starts again where its account is after the switch:
/// that account's folder, not ~/.claude, which holds the new one.
#[tokio::test]
async fn a_pinned_pane_slow_to_stop_comes_back_on_its_folder() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("late-pinned");
    w.with_accounts(&["Personal"], Some(vec![])).await;
    let q = windows::create(&w.ctx, window("q", claude("default", false)))
        .await
        .unwrap()
        .pane_id;
    let session = w.claude(&q).await.session_id.unwrap();
    let pid = w.claude_pid(&q).await;
    std::fs::write(w.sandbox.dir.join(format!("state/{pid}.linger")), "14").unwrap();

    let switched = w.global("personal").await.unwrap();
    assert!(switched.restarted.is_empty(), "{switched:?}");
    assert_eq!(w.accounts().registry().global, "personal");
    let info = w.settled_on(&q, "default").await;
    assert_eq!(info.follow, Follow::Pinned);
    assert_eq!(info.session_id.as_deref(), Some(session.as_str()));
    let started = w.start_command(&q);
    assert!(
        started.contains(&format!(
            "CLAUDE_CONFIG_DIR={}",
            w.paths.account_dir("default").display()
        )),
        "{started}"
    );
    assert_eq!(w.ask(&q, "who").await, "work");
}

/// A pane on the new account's folder whose conversation can't be made
/// visible on ~/.claude starts there anyway, with a new conversation, and
/// says so: never again on its folder, whose sign-in went to ~/.claude.
#[tokio::test]
async fn a_pane_whose_conversation_cant_follow_starts_a_new_one() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("no-share");
    w.with_accounts(&["Personal"], Some(vec![])).await;
    let p = windows::create(&w.ctx, window("p", claude("personal", false)))
        .await
        .unwrap()
        .pane_id;
    let session = w.claude(&p).await.session_id.unwrap();
    // A longer copy of that conversation on ~/.claude, which a move never
    // overwrites.
    let source = walk(&w.paths.account_dir("personal").join("projects"))
        .into_iter()
        .find(|file| file.file_name().and_then(|n| n.to_str()) == Some(&format!("{session}.jsonl")))
        .expect("its transcript");
    let project = source.parent().unwrap().file_name().unwrap();
    let target = w
        .paths
        .claude_home
        .join("projects")
        .join(project)
        .join(format!("{session}.jsonl"));
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let mut longer = std::fs::read_to_string(&source).unwrap();
    longer.push_str(&"{\"type\":\"padding\"}\n".repeat(400));
    std::fs::write(&target, longer).unwrap();

    let switched = w.global("personal").await.unwrap();
    assert_eq!(switched.restarted, vec![p.clone()], "{switched:?}");
    let info = w.settled_on_slot(&p, "personal").await;
    assert_ne!(info.session_id.as_deref(), Some(session.as_str()));
    let why = info.move_error.expect("it says why");
    assert!(
        why.contains(tmux::accounts::moves::NEWER_COPY) && why.contains("new conversation"),
        "{why}"
    );
    assert!(!w.start_command(&p).contains("--resume"));
    assert_eq!(w.ask(&p, "who").await, "personal");
}

/// Claude Code started by hand on the new account's folder whose shell
/// does not take the terminal back once it stops: the switch goes on (that
/// folder is free), and the pane says it was stopped and not started again.
#[tokio::test]
async fn a_shell_that_does_not_come_back_does_not_block_the_switch() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("shell-gone");
    w.with_accounts(&["Personal"], None).await;
    let shell = windows::create(&w.ctx, window("shell", None))
        .await
        .unwrap()
        .pane_id;
    let typed = format!(
        "env CLAUDE_CONFIG_DIR={} claude; sleep 30",
        w.paths.account_dir("personal").display()
    );
    w.tmux(&["send-keys", "-t", &shell, "-l", &typed]);
    w.tmux(&["send-keys", "-t", &shell, "Enter"]);
    let info = w.claude(&shell).await;
    assert_eq!(info.account.as_deref(), Some("personal"));
    assert!(info.movable, "{info:?}");

    let switched = w.global("personal").await.unwrap();
    assert_eq!(switched.state, account_fns::GlobalState::Switched);
    assert!(switched.restarted.is_empty(), "{switched:?}");
    assert_eq!(w.accounts().registry().global, "personal");
    let info = w.pane(&shell).await.unwrap().claude.expect("still shown");
    assert_eq!(
        info.move_error.as_deref(),
        Some(tmux::accounts::moves::SHELL_NOT_BACK)
    );
    assert!(!w.screen(&shell).contains("Resumed"));
}

/// Moves, removals and new Claude Code panes wait while a switch runs
/// (the swap gate): none of them acts on the stores half-way.
#[tokio::test]
async fn moves_removals_and_starts_wait_for_a_running_switch() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("gate");
    w.with_accounts(&["Personal", "Side"], None).await;
    let a = windows::create(&w.ctx, window("a", claude("global", false)))
        .await
        .unwrap()
        .pane_id;
    w.claude(&a).await;
    let short = Duration::from_millis(800);
    let gate = w.accounts().swap_gate.lock().await;
    let account = panes::account(
        &w.ctx,
        panes::PaneAccountInput {
            pane: a.clone(),
            account: "global".into(),
        },
    );
    assert!(tokio::time::timeout(short, account).await.is_err());
    let idle = account_fns::move_idle(
        &w.ctx,
        account_fns::MoveIdleInput {
            from: "side".into(),
            to: "personal".into(),
        },
    );
    assert!(tokio::time::timeout(short, idle).await.is_err());
    let remove = account_fns::remove(
        &w.ctx,
        account_fns::RemoveAccountInput {
            account: "side".into(),
            delete_files: false,
        },
    );
    assert!(tokio::time::timeout(short, remove).await.is_err());
    let start = windows::create(&w.ctx, window("b", claude("personal", false)));
    assert!(tokio::time::timeout(short, start).await.is_err());
    assert!(w.accounts().registry().has("side"));
    drop(gate);
    assert_eq!(
        pane_account(&w, &a, "global").await.state,
        panes::PaneAccountState::Following
    );
    account_fns::remove(
        &w.ctx,
        account_fns::RemoveAccountInput {
            account: "side".into(),
            delete_files: false,
        },
    )
    .await
    .unwrap();
    assert!(!w.accounts().registry().has("side"));
}

/// Before tmux 3.0 a pane's marks are its window's: a shell beside a Claude
/// Code pane carries that pane's marks, and must neither be moved nor drop
/// the move that pane waits for.
#[tokio::test]
async fn a_shell_beside_claude_leaves_its_move_alone() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("shared-shell");
    w.with_accounts(&["Personal"], None).await;
    let a = windows::create(&w.ctx, window("a", claude("global", false)))
        .await
        .unwrap()
        .pane_id;
    w.claude(&a).await;
    let split = std::process::Command::new(w.sandbox.dir.join("tmux"))
        .args([
            "-L",
            &w.sandbox.socket,
            "split-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
        ])
        .args(["-t", &a, "/bin/sh"])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .unwrap();
    let b = String::from_utf8_lossy(&split.stdout).trim().to_string();
    assert!(b.starts_with('%'), "{split:?}");
    w.set_state(&a, "working").await;
    let asked = pane_account(&w, &a, "personal").await;
    assert_eq!(asked.state, panes::PaneAccountState::AfterTurn);
    let refused = panes::account(
        &w.ctx,
        panes::PaneAccountInput {
            pane: b.clone(),
            account: "global".into(),
        },
    )
    .await
    .unwrap_err();
    assert!(refused.contains("not running"), "{refused}");
    for _ in 0..2 {
        w.pending_tick().await;
    }
    let info = w.pane(&a).await.unwrap().claude.unwrap();
    assert_eq!(info.pending.as_deref(), Some("personal"), "{info:?}");
    w.set_state(&a, "idle").await;
    w.pending_tick().await;
    w.settled_on(&a, "personal").await;
}

/// A call that goes away mid-switch (the SDK drops the handler on a
/// timeout or a disconnect) leaves the switch to finish holding the swap
/// gate: nothing else enters meanwhile, a removal of the account waits and
/// then finds it global, and the pane comes back on ~/.claude.
#[tokio::test]
async fn a_switch_whose_caller_goes_away_finishes_holding_the_gate() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("dropped");
    w.with_accounts(&["Personal"], Some(vec![])).await;
    let p = windows::create(&w.ctx, window("p", claude("personal", false)))
        .await
        .unwrap()
        .pane_id;
    let session = w.claude(&p).await.session_id.unwrap();
    // Its SessionEnd hooks take three seconds: the switch is still stopping
    // it when the call goes away.
    let pid = w.claude_pid(&p).await;
    std::fs::write(w.sandbox.dir.join(format!("state/{pid}.linger")), "3").unwrap();
    let cut = tokio::time::timeout(Duration::from_millis(700), w.global("personal")).await;
    assert!(cut.is_err(), "the call was cut off mid-switch");
    assert!(
        w.accounts().swap_gate.try_lock().is_err(),
        "the switch still holds the gate"
    );
    // Deleting Personal's folder waits for the switch, then is refused:
    // Personal is global by then.
    let refused = account_fns::remove(
        &w.ctx,
        account_fns::RemoveAccountInput {
            account: "personal".into(),
            delete_files: true,
        },
    )
    .await
    .unwrap_err();
    assert!(refused.contains("global account"), "{refused}");
    assert_eq!(w.accounts().registry().global, "personal");
    assert!(w.paths.account_dir("personal").is_dir());
    let info = w.settled_on_slot(&p, "personal").await;
    assert_eq!(info.session_id.as_deref(), Some(session.as_str()));
    assert_eq!(w.ask(&p, "who").await, "personal");
}

#[tokio::test]
async fn a_launch_whose_caller_goes_away_still_starts_claude() {
    if !available() {
        eprintln!("tmux or python3 not installed; skipping");
        return;
    }
    let w = world("launch-dropped");
    w.with_accounts(&["Personal"], Some(vec![])).await;
    // The worker's tmux takes two seconds to show an environment while
    // `slow` is there: the start on the slot looks at one first.
    let wrapper = w.sandbox.dir.join("tmux");
    let slow = w.sandbox.dir.join("slow");
    let script = std::fs::read_to_string(&wrapper).unwrap().replacen(
        "export PATH\n",
        &format!(
            "export PATH\ncase \" $* \" in *\" show-environment \"*) [ -e {} ] && sleep 2;; esac\n",
            sh_quote(&slow.to_string_lossy())
        ),
        1,
    );
    std::fs::write(&wrapper, script).unwrap();
    std::fs::write(&slow, "").unwrap();
    let cut = tokio::time::timeout(
        Duration::from_millis(800),
        windows::create(&w.ctx, window("dropped", claude("global", false))),
    )
    .await;
    assert!(cut.is_err(), "the call was cut off mid-start");
    std::fs::remove_file(&slow).unwrap();
    let pane = w.tmux_out(&["list-panes", "-t", "base:dropped", "-F", "#{pane_id}"]);
    assert!(pane.starts_with('%'), "the window was made: {pane:?}");
    // Claude Code starts in it all the same, never tmux's placeholder left.
    let info = w.claude(&pane).await;
    assert_eq!(info.account.as_deref(), Some("default"));
    assert_ne!(
        w.tmux_out(&[
            "display-message",
            "-p",
            "-t",
            &pane,
            "#{pane_current_command}"
        ]),
        "cat"
    );
}
