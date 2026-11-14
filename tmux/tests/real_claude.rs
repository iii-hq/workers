//! The login swap against the real Claude Code (agent-accounts G1, G3):
//! an idle session on `~/.claude` sends the new account's token with its
//! next request after `tmux::accounts::global`, with no restart.
//!
//! Runs only inside a throwaway container started with `--network none`
//! (`/.dockerenv`, `REAL_CLAUDE_CONTAINER=1`, nothing but the loopback
//! interface) with `REAL_CLAUDE_BIN` naming the binary; anywhere else it
//! skips. Everything it writes is in a sandbox folder: two fake sign-ins
//! (made-up tokens), a loopback mock of the Messages API that notes a
//! SHA-256 prefix of the token of each request (never the token), and an
//! isolated tmux server.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tmux::accounts::Accounts;
use tmux::accounts::credentials::FileStore;
use tmux::accounts::paths::ClaudePaths;
use tmux::collector::{Collector, ConsoleClients};
use tmux::config::WorkerConfig;
use tmux::events::Monitor;
use tmux::functions::accounts::{self as account_fns, ClaudeLaunchInput};
use tmux::functions::{self, Ctx, EmptyInput, windows};
use tmux::model::{Follow, Pane};

const TOKEN_A: &str = "sk-ant-oat01-fake-work-0000000000000000000000000000";
const REFRESH_A: &str = "sk-ant-ort01-fake-work-0000000000000000000000000000";
const TOKEN_B: &str = "sk-ant-oat01-fake-personal-00000000000000000000000";
const REFRESH_B: &str = "sk-ant-ort01-fake-personal-00000000000000000000000";

const MOCK: &str = r#"
import hashlib, http.server, json, sys
log = open(sys.argv[2], "a", buffering=1)

class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"

    def log_message(self, *args):
        pass

    def token(self):
        auth = self.headers.get("Authorization") or ""
        token = auth[7:] if auth.startswith("Bearer ") else (self.headers.get("x-api-key") or "")
        return hashlib.sha256(token.encode()).hexdigest()[:12] if token else "none"

    def note(self, method, model=None):
        log.write(json.dumps({"method": method, "path": self.path.split("?")[0],
                              "token": self.token(), "model": model}) + "\n")

    def reply(self, status, body, kind="application/json"):
        data = body.encode() if isinstance(body, str) else body
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        self.note("GET")
        self.reply(404, json.dumps({"type": "error", "error": {"type": "not_found_error", "message": "mock"}}))

    def do_POST(self):
        raw = self.rfile.read(int(self.headers.get("Content-Length") or 0))
        try:
            body = json.loads(raw or b"{}")
        except ValueError:
            body = {}
        path = self.path.split("?")[0]
        model = body.get("model", "claude-mock")
        self.note("POST", model)
        if path.endswith("/count_tokens"):
            return self.reply(200, json.dumps({"input_tokens": 12}))
        if not path.startswith("/v1/messages"):
            return self.reply(404, json.dumps({"type": "error", "error": {"type": "not_found_error", "message": "mock"}}))
        text = "Hi from the mock."
        usage = {"input_tokens": 12, "output_tokens": 5, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0}
        message = {"id": "msg_mock", "type": "message", "role": "assistant", "model": model,
                   "content": [], "stop_reason": None, "stop_sequence": None, "usage": usage}
        if not body.get("stream"):
            message.update({"content": [{"type": "text", "text": text}], "stop_reason": "end_turn"})
            return self.reply(200, json.dumps(message))
        events = [
            ("message_start", {"type": "message_start", "message": message}),
            ("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            ("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}}),
            ("content_block_stop", {"type": "content_block_stop", "index": 0}),
            ("message_delta", {"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": None}, "usage": {"output_tokens": 5}}),
            ("message_stop", {"type": "message_stop"}),
        ]
        stream = "".join("event: %s\ndata: %s\n\n" % (name, json.dumps(data)) for name, data in events)
        self.reply(200, stream, "text/event-stream")

http.server.ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
"#;

fn hash(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// The real binary, only where it may run: in a container without a network.
fn real_claude() -> Option<PathBuf> {
    let bin = std::env::var_os("REAL_CLAUDE_BIN")?;
    if !Path::new("/.dockerenv").exists()
        || std::env::var("REAL_CLAUDE_CONTAINER").as_deref() != Ok("1")
    {
        return None;
    }
    let mut interfaces: Vec<String> = std::fs::read_dir("/sys/class/net")
        .expect("network interfaces listed")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    interfaces.sort();
    assert_eq!(
        interfaces,
        vec!["lo".to_string()],
        "the real Claude Code runs only in a container with --network none"
    );
    Some(PathBuf::from(bin))
}

struct Sandbox {
    dir: PathBuf,
    socket: String,
    mock: Option<std::process::Child>,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::process::Command::new("tmux")
            .args(["-L", &self.socket, "kill-server"])
            .env_remove("TMUX")
            .output();
        if let Some(mock) = self.mock.as_mut() {
            let _ = mock.kill();
            let _ = mock.wait();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn write_json(path: &Path, value: &serde_json::Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

fn chain(token: &str, refresh: &str) -> serde_json::Value {
    serde_json::json!({"claudeAiOauth": {
        "accessToken": token, "refreshToken": refresh, "expiresAt": 4_102_444_800_000u64,
        "scopes": ["user:inference", "user:profile", "user:sessions:claude_code"],
        "subscriptionType": "max", "rateLimitTier": "default_claude_max_20x"}})
}

fn profile(uuid: &str, email: &str) -> serde_json::Value {
    serde_json::json!({"accountUuid": uuid, "emailAddress": email,
        "organizationUuid": format!("org-{uuid}"), "organizationName": "Mock",
        "organizationRole": "admin", "workspaceRole": null, "displayName": "You",
        "organizationType": "claude_max", "billingType": "stripe_subscription"})
}

fn log_lines(file: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn messages_after(file: &Path, from: usize) -> Vec<String> {
    log_lines(file)
        .into_iter()
        .skip(from)
        .filter(|entry| {
            entry["method"] == "POST"
                && entry["path"]
                    .as_str()
                    .is_some_and(|p| p.starts_with("/v1/messages") && !p.ends_with("count_tokens"))
        })
        .map(|entry| entry["token"].as_str().unwrap_or_default().to_string())
        .collect()
}

async fn find_pane(ctx: &Ctx, pane_id: &str) -> Option<Pane> {
    functions::sessions_list(ctx, EmptyInput {})
        .await
        .ok()?
        .sessions
        .into_iter()
        .flat_map(|session| session.windows)
        .flat_map(|window| window.panes)
        .find(|pane| pane.id == pane_id)
}

async fn wait_for<T>(what: &str, secs: u64, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(value) = probe() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[tokio::test]
async fn an_idle_session_uses_the_new_account_after_a_swap() {
    let Some(claude) = real_claude() else {
        eprintln!("not in the real-Claude container; skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("real-claude-{}", uuid::Uuid::new_v4().simple()));
    let mut sandbox = Sandbox {
        dir: dir.clone(),
        socket: format!("iii-real-claude-{}", uuid::Uuid::new_v4().simple()),
        mock: None,
    };
    let home = dir.join("home");
    let project = home.join("project");
    let bin = dir.join("bin");
    for folder in [home.join(".claude"), project.clone(), bin.clone()] {
        std::fs::create_dir_all(folder).unwrap();
    }
    std::os::unix::fs::symlink(&claude, bin.join("claude")).unwrap();

    // The mock of the Messages API, on loopback.
    let port = 41000 + (std::process::id() % 2000) as u16;
    let mock_py = dir.join("mock.py");
    let log = dir.join("requests.log");
    std::fs::write(&mock_py, MOCK).unwrap();
    sandbox.mock = Some(
        std::process::Command::new("python3")
            .arg(&mock_py)
            .arg(port.to_string())
            .arg(&log)
            .spawn()
            .unwrap(),
    );
    wait_for("the mock", 10, || {
        std::net::TcpStream::connect(("127.0.0.1", port)).ok()
    })
    .await;

    // Work signed in to ~/.claude, onboarding done, the project trusted.
    write_json(
        &home.join(".claude/.credentials.json"),
        &chain(TOKEN_A, REFRESH_A),
    );
    let project_key = project
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    write_json(
        &home.join(".claude.json"),
        &serde_json::json!({
            "oauthAccount": profile("00000000-0000-4000-8000-00000000000a", "work@mock.dev"),
            "hasCompletedOnboarding": true, "lastOnboardingVersion": "2.1.322",
            "theme": "dark", "numStartups": 5, "hasSeenTasksHint": true,
            "hasAcknowledgedCostThreshold": true, "bypassPermissionsModeAccepted": true,
            "autoUpdates": false, "installMethod": "native",
            "projects": {project_key.clone(): {"hasTrustDialogAccepted": true,
                "hasCompletedProjectOnboarding": true, "allowedTools": [],
                "projectOnboardingSeenCount": 1}},
        }),
    );
    write_json(
        &home.join(".claude/settings.json"),
        &serde_json::json!({"autoUpdates": false}),
    );

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
    ]);
    let paths = ClaudePaths::from_vars(|key| vars.get(key).cloned(), false).unwrap();
    let accounts = Arc::new(Accounts::new(
        Some(paths.clone()),
        Box::new(FileStore),
        false,
    ));

    // tmux, with the real binary first on PATH and Claude Code pointed at
    // the mock: the server's environment and, through a wrapper, that of
    // every tmux command the worker runs (tmux hands a new window the
    // asking client's PATH).
    let path = format!("{}:/usr/local/bin:/usr/bin:/bin", bin.display());
    let base_url = format!("http://127.0.0.1:{port}");
    let wrapper = dir.join("tmux");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexport HOME='{}' PATH='{}' ANTHROPIC_BASE_URL='{}' DISABLE_TELEMETRY=1 CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 DISABLE_AUTOUPDATER=1 DISABLE_ERROR_REPORTING=1\nunset CLAUDE_CONFIG_DIR TMUX TMUX_PANE\nexec /usr/local/bin/tmux \"$@\"\n",
            home.display(),
            path,
            base_url
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let started = std::process::Command::new(&wrapper)
        .args([
            "-L",
            &sandbox.socket,
            "-f",
            "/dev/null",
            "new-session",
            "-d",
        ])
        .args(["-s", "base", "-x", "200", "-y", "50", "/bin/sh"])
        .args([";", "set-option", "-g", "default-shell", "/bin/sh"])
        .env_clear()
        .env("HOME", &home)
        .env("PATH", &path)
        .env("SHELL", "/bin/sh")
        .env("TERM", "xterm-256color")
        .env("LANG", "en_US.UTF-8")
        .env("ANTHROPIC_BASE_URL", &base_url)
        .env("DISABLE_TELEMETRY", "1")
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .env("DISABLE_AUTOUPDATER", "1")
        .env("DISABLE_ERROR_REPORTING", "1")
        .current_dir(&home)
        .output()
        .unwrap();
    assert!(started.status.success(), "{started:?}");
    let tmux_out = |args: &[&str]| {
        let output = std::process::Command::new("tmux")
            .arg("-L")
            .arg(&sandbox.socket)
            .args(args)
            .env_remove("TMUX")
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).to_string()
    };

    let config = WorkerConfig {
        socket_name: Some(sandbox.socket.clone()),
        tmux_binary: wrapper.to_string_lossy().into_owned(),
        ..WorkerConfig::default()
    }
    .into_shared();
    let collector = Arc::new(Collector::with_accounts(
        config,
        ConsoleClients::default(),
        accounts.clone(),
    ));
    let monitor = Monitor::new(
        iii_sdk::IIIClient::new("ws://stub-not-connected:0"),
        collector.clone(),
    );
    let ctx = Ctx {
        collector: collector.clone(),
        monitor,
        transcripts: Default::default(),
    };

    // A second account, Personal, signed in on its folder.
    account_fns::add(
        &ctx,
        account_fns::AddAccountInput {
            name: "Personal".into(),
            color: "green".into(),
            shared: None,
        },
    )
    .await
    .unwrap();
    write_json(
        &paths.account_dir("personal").join(".credentials.json"),
        &chain(TOKEN_B, REFRESH_B),
    );
    write_json(
        &paths.account_json("personal"),
        &serde_json::json!({
            "oauthAccount": profile("00000000-0000-4000-8000-00000000000b", "personal@mock.dev"),
            "hasCompletedOnboarding": true, "theme": "dark",
            "projects": {project_key: {"hasTrustDialogAccepted": true}},
        }),
    );
    assert_eq!(accounts.registry().global, "default");

    // Claude Code on the global account, as the worker starts it.
    let created = windows::create(
        &ctx,
        windows::CreateWindowInput {
            session: "base".into(),
            name: Some("claude".into()),
            cwd: Some(project.to_string_lossy().into_owned()),
            command: None,
            select: Some(true),
            remain_on_exit: true,
            claude: Some(ClaudeLaunchInput {
                account: "global".into(),
                skip_permissions: false,
            }),
        },
    )
    .await
    .unwrap();
    assert_eq!(created.runs.as_deref(), Some("claude"));
    let pane_id = created.pane_id.clone();
    let screen = || tmux_out(&["capture-pane", "-p", "-t", &pane_id]);

    // Its session record, idle: the worker sees it on the slot, following.
    let deadline = Instant::now() + Duration::from_secs(60);
    let first: Pane = loop {
        if let Some(found) = find_pane(&ctx, &pane_id).await
            && found
                .claude
                .as_ref()
                .is_some_and(|c| c.session_id.is_some())
        {
            break found;
        }
        assert!(
            Instant::now() < deadline,
            "Claude Code never recorded its session; screen:\n{}",
            screen()
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    let info = first.claude.clone().unwrap();
    eprintln!("first look: {info:?}");
    assert_eq!(info.follow, Follow::Global);
    assert_eq!(info.account.as_deref(), Some("default"));
    let claude_pid = std::fs::read_dir(home.join(".claude/sessions"))
        .unwrap()
        .filter_map(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .strip_suffix(".json")
                .and_then(|pid| pid.parse::<u32>().ok())
        })
        .next()
        .expect("a session record");
    // Let start-up settle (its own requests, if any).
    tokio::time::sleep(Duration::from_secs(4)).await;
    let startup = log_lines(&log);
    eprintln!("start-up requests: {startup:?}");

    let ask = |text: &str| {
        tmux_out(&["send-keys", "-t", &pane_id, "-l", text]);
        std::thread::sleep(Duration::from_millis(600));
        tmux_out(&["send-keys", "-t", &pane_id, "Enter"]);
    };

    // A message as Work.
    let before = log_lines(&log).len();
    ask("Say hi");
    let tokens = wait_for("a request as Work", 60, || {
        let seen = messages_after(&log, before);
        (!seen.is_empty()).then_some(seen)
    })
    .await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let tokens_a = messages_after(&log, before);
    eprintln!(
        "requests for the first message: {tokens_a:?} (Work {}, Personal {})",
        hash(TOKEN_A),
        hash(TOKEN_B)
    );
    assert!(!tokens.is_empty());
    assert!(
        tokens_a.iter().all(|t| *t == hash(TOKEN_A)),
        "the first message went as Work: {tokens_a:?}; screen:\n{}",
        screen()
    );

    // The switch, while Claude Code sits idle.
    wait_for("Claude Code idle", 60, || {
        let record =
            std::fs::read_to_string(home.join(format!(".claude/sessions/{claude_pid}.json")))
                .ok()?;
        let value: serde_json::Value = serde_json::from_str(&record).ok()?;
        (value["status"] == "idle").then_some(())
    })
    .await;
    let switched = account_fns::global(
        &ctx,
        account_fns::GlobalInput {
            account: "personal".into(),
        },
    )
    .await
    .unwrap();
    eprintln!("switch: {switched:?}");
    assert_eq!(switched.global, "personal");
    assert_eq!(switched.following, 1, "the real Claude Code follows");
    assert!(switched.restarted.is_empty());

    // The next message, from the same process, goes as Personal.
    let before = log_lines(&log).len();
    ask("Say hi again");
    wait_for("a request as Personal", 60, || {
        let seen = messages_after(&log, before);
        (!seen.is_empty()).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let tokens_b = messages_after(&log, before);
    eprintln!("requests for the second message: {tokens_b:?}");
    eprintln!("screen:\n{}", screen());
    assert!(
        tokens_b.iter().all(|t| *t == hash(TOKEN_B)),
        "after the switch every request goes as Personal: {tokens_b:?}"
    );
    // No restart: the same Claude Code process, still running.
    assert!(
        Path::new(&format!("/proc/{claude_pid}")).exists(),
        "Claude Code was restarted"
    );
    let after = find_pane(&ctx, &pane_id).await.unwrap().claude.unwrap();
    assert_eq!(after.session_id, info.session_id);
    assert_eq!(after.account.as_deref(), Some("personal"));
    assert_eq!(after.follow, Follow::Global);
    // Claude Code kept Personal's profile in ~/.claude.json (no write-back
    // of Work's) and its sign-in where the swap left it.
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".claude.json")).unwrap()).unwrap();
    eprintln!(
        "slot profile now: {}",
        config["oauthAccount"]["emailAddress"]
    );
    let slot: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.join(".claude/.credentials.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        hash(slot["claudeAiOauth"]["refreshToken"].as_str().unwrap()),
        hash(REFRESH_B)
    );
    let work: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(paths.account_dir("default").join(".credentials.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        hash(work["claudeAiOauth"]["refreshToken"].as_str().unwrap()),
        hash(REFRESH_A)
    );
    assert_eq!(
        config["oauthAccount"]["accountUuid"], "00000000-0000-4000-8000-00000000000b",
        "the profile stayed Personal's"
    );

    // And back: Work's sign-in returns to ~/.claude, Personal's to its folder.
    let back = account_fns::global(
        &ctx,
        account_fns::GlobalInput {
            account: "default".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!((back.global.as_str(), back.following), ("default", 1));
    let before = log_lines(&log).len();
    ask("Once more");
    wait_for("a request as Work again", 60, || {
        let seen = messages_after(&log, before);
        (!seen.is_empty()).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let tokens_c = messages_after(&log, before);
    eprintln!("requests for the third message: {tokens_c:?}");
    assert!(
        tokens_c.iter().all(|t| *t == hash(TOKEN_A)),
        "back on Work: {tokens_c:?}"
    );
    assert!(
        Path::new(&format!("/proc/{claude_pid}")).exists(),
        "Claude Code was restarted"
    );
    let personal: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(paths.account_dir("personal").join(".credentials.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        hash(personal["claudeAiOauth"]["refreshToken"].as_str().unwrap()),
        hash(REFRESH_B)
    );
    eprintln!("all requests: {:?}", log_lines(&log));
}
