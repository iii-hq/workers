//! `tmux::accounts::login_start` / `login_code` against the real Claude Code
//! (claude-login-ui WD1–WD6): the link is read from what it prints, it
//! opens no browser with `browser: none` (a trap `xdg-open`/`open` first
//! on PATH and `DISPLAY` set, so without the worker's `BROWSER` it would
//! call the trap), it opens the worker's opener with `browser: host`, and a
//! made-up code comes back `rejected` with Claude Code's own sentence.
//!
//! It needs Anthropic's sign-in endpoint for the rejection, so it runs only
//! where it is asked to with `REAL_CLAUDE_LOGIN=1` and `REAL_CLAUDE_BIN`
//! naming the binary: inside a throwaway container (`/.dockerenv`), or on a
//! Mac with `REAL_CLAUDE_LOGIN_MAC=1` too (macOS opens links with `open`,
//! which the worker must keep from running); anywhere else it skips.
//! Everything Claude Code writes is in a sandbox folder (`CLAUDE_CONFIG_DIR`
//! and `HOME`); no real code is ever typed, so nothing is ever signed in.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tmux::accounts::credentials::FileStore;
use tmux::accounts::login::Runner;
use tmux::accounts::paths::ClaudePaths;
use tmux::accounts::Accounts;
use tmux::collector::{Collector, ConsoleClients};
use tmux::config::WorkerConfig;
use tmux::events::Monitor;
use tmux::functions::accounts as account_fns;
use tmux::functions::{Ctx, EmptyInput};
use tmux::model::{LoginBrowser, LoginState};

fn real_claude() -> Option<PathBuf> {
    let bin = std::env::var_os("REAL_CLAUDE_BIN")?;
    let asked = |key: &str| std::env::var(key).as_deref() == Ok("1");
    let here = Path::new("/.dockerenv").exists()
        || (cfg!(target_os = "macos") && asked("REAL_CLAUDE_LOGIN_MAC"));
    (here && asked("REAL_CLAUDE_LOGIN")).then(|| PathBuf::from(bin))
}

fn script(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[tokio::test]
async fn the_real_claude_code_signs_in_through_the_worker_s_pty() {
    let Some(claude) = real_claude() else {
        eprintln!("not in a REAL_CLAUDE_LOGIN container; skipping");
        return;
    };
    let dir =
        std::env::temp_dir().join(format!("tmux-real-login-{}", uuid::Uuid::new_v4().simple()));
    let home = dir.join("home");
    let traps = dir.join("traps");
    for folder in [home.join(".claude"), traps.clone()] {
        std::fs::create_dir_all(folder).unwrap();
    }
    // What Claude Code would run to open a browser without the worker's
    // BROWSER: noted, never a browser.
    let trapped = dir.join("trapped.log");
    for name in ["xdg-open", "open", "opener"] {
        script(
            &traps.join(name),
            &format!("#!/bin/sh\necho \"{name} $1\" >> '{}'\n", trapped.display()),
        );
    }
    // SAFETY: the only test in this binary; nothing else reads the
    // environment yet.
    std::env::set_var(
        "PATH",
        format!(
            "{}:{}",
            traps.display(),
            std::env::var("PATH").unwrap_or_default()
        ),
    );
    let vars: std::collections::HashMap<&str, String> = std::collections::HashMap::from([
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
    accounts.logins.set_runner(Runner {
        claude: Some(claude),
        env: vec![
            // A display: xdg-open would be tried without the worker's BROWSER.
            ("DISPLAY".into(), ":0".into()),
            ("DISABLE_TELEMETRY".into(), "1".into()),
            ("DISABLE_AUTOUPDATER".into(), "1".into()),
        ],
        opener: Some(traps.join("opener")),
        ttl_ms: None,
    });
    let collector = Arc::new(Collector::with_accounts(
        WorkerConfig {
            socket_name: Some(format!("iii-tmux-real-{}", uuid::Uuid::new_v4().simple())),
            ..WorkerConfig::default()
        }
        .into_shared(),
        ConsoleClients::default(),
        accounts.clone(),
    ));
    let ctx = Ctx {
        monitor: Monitor::new(
            iii_sdk::IIIClient::new("ws://stub-not-connected:0"),
            collector.clone(),
        ),
        collector,
        transcripts: Default::default(),
    };
    let id = account_fns::add(
        &ctx,
        account_fns::AddAccountInput {
            name: "Personal".into(),
            color: "green".into(),
            shared: Some(vec![]),
        },
    )
    .await
    .unwrap()
    .account
    .id;

    // browser: none.
    let started = account_fns::login_start(
        &ctx,
        account_fns::LoginStartInput {
            account: id.clone(),
            browser: None,
        },
    )
    .await
    .unwrap();
    let url = &started.url;
    eprintln!("link: {}", tmux::accounts::login::redact_url(url));
    assert!(
        url.starts_with("https://") && url.contains("/oauth/authorize?"),
        "{url}"
    );
    assert!(
        url.contains("redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback"),
        "{url}"
    );
    assert!(
        url.contains("code_challenge=") && url.contains("&state="),
        "{url}"
    );
    assert_eq!(started.browser_url, None);
    // Claude Code handed the browser link to the worker's shim (noted in
    // the login's folder), and nothing else saw it.
    let noted = dir_of(&started.login_id).join("browser-link");
    let mut waited = 0;
    while !noted.exists() && waited < 50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        waited += 1;
    }
    let link = std::fs::read_to_string(&noted).expect("the shim noted the browser link");
    assert!(
        link.contains("redirect_uri=http%3A%2F%2Flocalhost%3A"),
        "{link}"
    );
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        !trapped.exists(),
        "a browser opener ran: {:?}",
        std::fs::read_to_string(&trapped)
    );

    // Not a code: the worker answers.
    let error = account_fns::login_code(
        &ctx,
        account_fns::LoginCodeInput {
            login_id: started.login_id.clone(),
            code: "not a code".into(),
        },
    )
    .await
    .unwrap_err();
    assert!(error.contains("\"invalid_code\""), "{error}");
    // A made-up code: Anthropic refuses it, Claude Code ends.
    let error = account_fns::login_code(
        &ctx,
        account_fns::LoginCodeInput {
            login_id: started.login_id.clone(),
            code: "madeupcode0000000000000000000000#madeupstate0000000000000000000".into(),
        },
    )
    .await
    .unwrap_err();
    eprintln!("rejection: {error}");
    let error: serde_json::Value = serde_json::from_str(&error).unwrap();
    assert_eq!(error["code"], "rejected", "{error}");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .starts_with("That code wasn't accepted."),
        "{error}"
    );
    let listed = account_fns::list(&ctx, EmptyInput {}).await.unwrap();
    let entry = listed
        .logins
        .iter()
        .find(|login| login.login_id == started.login_id)
        .unwrap();
    assert_eq!(entry.state, LoginState::Failed);
    assert!(
        !listed
            .accounts
            .iter()
            .find(|a| a.id == id)
            .unwrap()
            .signed_in
    );
    assert!(!trapped.exists());

    // browser: host: the worker's opener gets the link that comes back to
    // Claude Code's loopback listener.
    let started = account_fns::login_start(
        &ctx,
        account_fns::LoginStartInput {
            account: id.clone(),
            browser: Some(LoginBrowser::Host),
        },
    )
    .await
    .unwrap();
    let browser_url = started.browser_url.clone().expect("browser_url");
    assert!(
        browser_url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A"),
        "{browser_url}"
    );
    let opened = std::fs::read_to_string(&trapped).unwrap();
    assert_eq!(opened.trim(), format!("opener {browser_url}"));
    account_fns::login_cancel(
        &ctx,
        account_fns::LoginCancelInput {
            login_id: started.login_id.clone(),
        },
    )
    .await
    .unwrap();
    accounts.logins.cancel_all();
    let _ = std::fs::remove_dir_all(&dir);
}

fn dir_of(login_id: &str) -> PathBuf {
    std::env::temp_dir().join(format!("iii-tmux-login-{login_id}"))
}
