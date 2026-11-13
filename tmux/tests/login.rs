//! Signing Claude Code accounts in from the app (claude-login-ui §2):
//! `tmux::accounts::login_start`, `login_code` and `login_cancel` with the
//! demo kit's fake Claude Code (its `auth login` behaves like 2.1.323's) in
//! a temporary home: never the developer's sign-in, browser or tmux. A
//! browser is a trap script that notes the link it was handed.
//! They need python3 (the fake is a Python script) and fail without it.
//! Every scenario removes its folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tmux::accounts::credentials::FileStore;
use tmux::accounts::login::Runner;
use tmux::accounts::paths::ClaudePaths;
use tmux::accounts::Accounts;
use tmux::collector::{Collector, ConsoleClients};
use tmux::config::WorkerConfig;
use tmux::events::Monitor;
use tmux::functions::accounts::{self as account_fns};
use tmux::functions::{Ctx, EmptyInput};
use tmux::model::{ClaudeLogin, LoginBrowser, LoginState};

struct World {
    dir: PathBuf,
    ctx: Ctx,
    paths: ClaudePaths,
    /// What the fake needs in its environment.
    env: Vec<(String, String)>,
}

impl Drop for World {
    fn drop(&mut self) {
        self.ctx.collector.accounts().logins.cancel_all();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The fake Claude Code is a Python script: without python3 these tests
/// can't say anything, so they fail rather than pass unchecked.
fn need_python() {
    let found = std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(found, "these tests need python3 (the fake Claude Code)");
}

fn script(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A home with Work signed in to `~/.claude` and accounts `others` (ids)
/// signed in on their folders, as the demo kit's people.
async fn world(name: &str, others: &[&str]) -> World {
    need_python();
    let dir = std::env::temp_dir().join(format!(
        "tmux-login-{name}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let home = dir.join("home");
    let bin = dir.join("bin");
    for folder in [home.join(".claude"), bin.clone(), dir.join("state")] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("demo/files/claude");
    std::fs::copy(
        fake.with_file_name("setup_demo.py"),
        bin.join("setup_demo.py"),
    )
    .unwrap();
    script(
        &bin.join("claude"),
        &std::fs::read_to_string(&fake).unwrap(),
    );
    // The browser on "this computer": notes what it was asked to open.
    script(
        &bin.join("trap-browser"),
        &format!(
            "#!/bin/sh\necho \"$1\" >> '{}'\n",
            dir.join("opened.log").display()
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
        (
            "CODEX_HOME",
            dir.join("codex").to_string_lossy().into_owned(),
        ),
    ]);
    let paths = ClaudePaths::from_vars(|key| vars.get(key).cloned(), false).unwrap();
    assert!(paths.claude_home.starts_with(&dir) && paths.root.starts_with(&dir));
    let accounts = Arc::new(Accounts::new(
        Some(paths.clone()),
        Box::new(FileStore),
        false,
    ));
    let sandbox_env = vec![
        (
            "FAKE_CLAUDE_SANDBOX".to_string(),
            dir.to_string_lossy().into_owned(),
        ),
        (
            "FAKE_CLAUDE_STATE_DIR".to_string(),
            dir.join("state").to_string_lossy().into_owned(),
        ),
    ];
    // Never the real Claude Code, nor a real browser.
    accounts.logins.set_runner(Runner {
        claude: Some(bin.join("claude")),
        env: sandbox_env.clone(),
        opener: Some(bin.join("trap-browser")),
        ..Runner::default()
    });
    let config = WorkerConfig {
        socket_name: Some(format!("iii-tmux-login-{}", uuid::Uuid::new_v4().simple())),
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
    let w = World {
        dir,
        ctx: Ctx {
            collector,
            monitor,
            transcripts: Default::default(),
        },
        paths,
        env: sandbox_env.clone(),
    };
    w.demo_sign_in(
        &w.paths.claude_home,
        &w.paths.default_json,
        "default",
        &sandbox_env,
    );
    for id in others {
        let name = match *id {
            "personal" => "Personal",
            "side-project" => "Side project",
            other => other,
        };
        let added = account_fns::add(
            &w.ctx,
            account_fns::AddAccountInput {
                name: name.into(),
                color: "green".into(),
                shared: Some(vec![]),
            },
        )
        .await
        .unwrap()
        .account;
        assert_eq!(&added.id, id);
        w.demo_sign_in(
            &w.paths.account_dir(id),
            &w.paths.account_json(id),
            id,
            &sandbox_env,
        );
    }
    w.ctx.collector.accounts().load_now().await;
    w
}

impl World {
    /// The fake and the trap browser as in [`world`], changed by `change`.
    fn runner(&self, change: impl FnOnce(&mut Runner)) {
        let mut runner = Runner {
            claude: Some(self.dir.join("bin/claude")),
            env: self.env.clone(),
            opener: Some(self.dir.join("bin/trap-browser")),
            ..Runner::default()
        };
        change(&mut runner);
        self.ctx.collector.accounts().logins.set_runner(runner);
    }

    /// The fake's pid for the latest `auth login`.
    fn fake_pid(&self) -> i32 {
        std::fs::read_to_string(self.dir.join("state/auth-login.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    /// What the fake's `auth login` leaves, through the demo kit itself.
    fn demo_sign_in(&self, dir: &Path, json: &Path, id: &str, env: &[(String, String)]) {
        let ran = std::process::Command::new("python3")
            .arg("-c")
            .arg(
                "import sys, setup_demo; setup_demo.sign_in(sys.argv[1], sys.argv[2], sys.argv[3])",
            )
            .args([dir.as_os_str(), json.as_os_str(), id.as_ref()])
            .current_dir(self.dir.join("bin"))
            .env("HOME", self.dir.join("home"))
            .envs(env.iter().map(|(k, v)| (k, v)))
            .output()
            .unwrap();
        assert!(ran.status.success(), "{ran:?}");
    }

    fn token_in(&self, dir: &Path) -> Option<String> {
        let text = std::fs::read_to_string(dir.join(".credentials.json")).ok()?;
        let value: serde_json::Value = serde_json::from_str(&text).ok()?;
        value["claudeAiOauth"]["accessToken"]
            .as_str()
            .map(|token| token.trim_start_matches("demo-").to_string())
    }

    async fn start(
        &self,
        account: &str,
        browser: Option<LoginBrowser>,
    ) -> account_fns::LoginStartOutput {
        account_fns::login_start(
            &self.ctx,
            account_fns::LoginStartInput {
                account: account.into(),
                browser,
            },
        )
        .await
        .unwrap()
    }

    async fn code(
        &self,
        login_id: &str,
        code: &str,
    ) -> Result<account_fns::LoginCodeOutput, serde_json::Value> {
        account_fns::login_code(
            &self.ctx,
            account_fns::LoginCodeInput {
                login_id: login_id.into(),
                code: code.into(),
            },
        )
        .await
        .map_err(|error| serde_json::from_str(&error).expect("errors are JSON objects"))
    }

    async fn logins(&self) -> Vec<ClaudeLogin> {
        account_fns::list(&self.ctx, EmptyInput {})
            .await
            .unwrap()
            .logins
    }

    async fn login(&self, id: &str) -> Option<ClaudeLogin> {
        self.logins()
            .await
            .into_iter()
            .find(|login| login.login_id == id)
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
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The happy path on an account's folder: the link, a shapeless paste
/// refused before Claude Code sees it, then the code; the feed follows.
#[tokio::test]
async fn a_code_signs_an_account_in_on_its_folder() {
    let w = world("folder", &["personal"]).await;
    // Signed out first, as after an expired sign-in.
    std::fs::remove_file(w.paths.account_dir("personal").join(".credentials.json")).unwrap();
    let started = w.start("personal", None).await;
    assert_eq!(started.account, "personal");
    assert_eq!(started.browser, LoginBrowser::None);
    assert!(
        started
            .url
            .starts_with("https://claude.com/cai/oauth/authorize?"),
        "{}",
        started.url
    );
    assert!(started
        .url
        .contains("redirect_uri=https%3A%2F%2Fplatform.claude.com"));
    // The account's email fills the page.
    assert_eq!(started.email.as_deref(), Some("you@home.dev"));
    assert!(started.url.contains("login_hint=you%40home.dev"));
    assert_eq!(started.browser_url, None);
    let span = started.expires_at_ms - tmux::model::now_ms();
    assert!((9 * 60_000..=10 * 60_000).contains(&span), "{span}");
    let entry = w.login(&started.login_id).await.unwrap();
    assert_eq!(entry.state, LoginState::WaitingCode);
    assert_eq!(entry.url, started.url);

    // Not a code: refused here, the login still waits.
    let error = w
        .code(&started.login_id, "https://claude.com/cai/oauth/x")
        .await
        .unwrap_err();
    assert_eq!(error["code"], "invalid_code");
    let error = w.code(&started.login_id, "   ").await.unwrap_err();
    assert_eq!(error["code"], "invalid_code");
    assert_eq!(
        w.login(&started.login_id).await.unwrap().state,
        LoginState::WaitingCode
    );

    // Whitespace from a copy is taken out.
    let done = w
        .code(&started.login_id, " demo-ok\n#demo-\tstate ")
        .await
        .unwrap();
    assert_eq!(done.account.id, "personal");
    assert!(done.account.signed_in);
    assert_eq!(done.account.email.as_deref(), Some("you@home.dev"));
    assert_eq!(
        w.token_in(&w.paths.account_dir("personal")).as_deref(),
        Some("personal")
    );
    let entry = w.login(&started.login_id).await.unwrap();
    assert_eq!(entry.state, LoginState::Done);
    assert_eq!(entry.error, None);
    // Asked again, the same answer.
    let again = w
        .code(&started.login_id, "demo-ok#demo-state")
        .await
        .unwrap();
    assert_eq!(again.account.id, "personal");
    // The worker's folder for it (the shims, the link) is gone.
    let leftovers: Vec<_> = std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .contains(&started.login_id)
        })
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

/// WD6: a wrong code ends Claude Code; the login is over (`rejected`).
#[tokio::test]
async fn a_wrong_code_is_rejected_and_ends_the_login() {
    let w = world("rejected", &["personal"]).await;
    let started = w.start("personal", None).await;
    let error = w
        .code(&started.login_id, "AbCdEf123#xyz")
        .await
        .unwrap_err();
    assert_eq!(error["code"], "rejected", "{error}");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .starts_with("That code wasn't accepted."),
        "{error}"
    );
    assert!(!error["message"].as_str().unwrap().contains("AbCdEf123"));
    let entry = w.login(&started.login_id).await.unwrap();
    assert_eq!(entry.state, LoginState::Failed);
    assert_eq!(entry.error.unwrap().code, "rejected");
    // The login is over: another code gets the same answer, not a hang.
    let again = w
        .code(&started.login_id, "demo-ok#demo-state")
        .await
        .unwrap_err();
    assert_eq!(again["code"], "rejected");
    // The account kept its sign-in.
    assert_eq!(
        w.token_in(&w.paths.account_dir("personal")).as_deref(),
        Some("personal")
    );
}

/// A new start for the same account cancels the one under way; cancel
/// stops a login, which leaves the feed; unknown logins are `gone`.
#[tokio::test]
async fn starting_again_or_cancelling_ends_the_login_under_way() {
    let w = world("cancel", &["personal", "side-project"]).await;
    let first = w.start("personal", None).await;
    let other = w.start("side-project", None).await;
    let second = w.start("personal", None).await;
    assert_ne!(first.login_id, second.login_id);
    let listed: Vec<String> = w.logins().await.into_iter().map(|l| l.login_id).collect();
    assert!(!listed.contains(&first.login_id), "{listed:?}");
    assert!(listed.contains(&second.login_id) && listed.contains(&other.login_id));
    assert_eq!(
        w.code(&first.login_id, "demo-ok#demo-state")
            .await
            .unwrap_err()["code"],
        "gone"
    );

    account_fns::login_cancel(
        &w.ctx,
        account_fns::LoginCancelInput {
            login_id: second.login_id.clone(),
        },
    )
    .await
    .unwrap();
    assert!(w.login(&second.login_id).await.is_none());
    // Its folder (the browser shim) is gone with it, and the replaced
    // login's too.
    for id in [&first.login_id, &second.login_id] {
        let folder = std::env::temp_dir().join(format!("iii-tmux-login-{id}"));
        assert!(!folder.exists(), "{}", folder.display());
    }
    assert_eq!(
        w.code(&second.login_id, "demo-ok#demo-state")
            .await
            .unwrap_err()["code"],
        "gone"
    );
    // Cancelling again, or something unknown: nothing to do.
    account_fns::login_cancel(
        &w.ctx,
        account_fns::LoginCancelInput {
            login_id: "nope".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        w.code("nope", "demo-ok#demo-state").await.unwrap_err()["code"],
        "gone"
    );
    // Unknown comes first: no shape to check for a sign-in that isn't.
    assert_eq!(
        w.code("nope", "not a code").await.unwrap_err()["code"],
        "gone"
    );
    // The other account's login goes on.
    assert!(w.code(&other.login_id, "demo-ok#demo-state").await.is_ok());
}

/// `browser: host`: Claude Code opens the browser here (a trap), and its
/// return to Claude Code's loopback listener signs in with no code.
#[tokio::test]
async fn with_the_browser_here_the_return_signs_in_by_itself() {
    let w = world("host", &["personal"]).await;
    std::fs::remove_file(w.paths.account_dir("personal").join(".credentials.json")).unwrap();
    let started = w.start("personal", Some(LoginBrowser::Host)).await;
    assert_eq!(started.browser, LoginBrowser::Host);
    let browser_url = started
        .browser_url
        .clone()
        .expect("the link Claude Code opened");
    assert!(
        browser_url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A"),
        "{browser_url}"
    );
    let opened = eventually("the browser opened", || async {
        std::fs::read_to_string(w.dir.join("opened.log")).ok()
    })
    .await;
    assert_eq!(opened.trim(), browser_url);
    // Only the answer to login_start carries it: the list (read without
    // approval) and the feed never do, since it signs in with no code.
    let listed = serde_json::to_string(&w.logins().await).unwrap();
    assert!(listed.contains(&started.login_id), "{listed}");
    assert!(
        !listed.contains("browser_url") && !listed.contains("localhost"),
        "{listed}"
    );
    // What the browser does once the person signed in.
    let port: u16 = browser_url
        .split("localhost%3A")
        .nth(1)
        .and_then(|rest| rest.split('%').next())
        .and_then(|port| port.parse().ok())
        .unwrap();
    let back = tokio::task::spawn_blocking(move || {
        use std::io::{Read, Write};
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .write_all(
                b"GET /callback?code=demo-ok&state=demo-state HTTP/1.0\r\nHost: localhost\r\n\r\n",
            )
            .unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        answer
    })
    .await
    .unwrap();
    assert!(back.starts_with("HTTP/1.0 200"), "{back}");
    let entry = eventually("signed in by itself", || async {
        w.login(&started.login_id)
            .await
            .filter(|l| l.state == LoginState::Done)
    })
    .await;
    assert_eq!(entry.error, None);
    assert_eq!(
        w.token_in(&w.paths.account_dir("personal")).as_deref(),
        Some("personal")
    );
    // A code pasted afterwards (another device) gets the same answer.
    assert_eq!(
        w.code(&started.login_id, "x#y").await.unwrap().account.id,
        "personal"
    );
    // `none` never reaches the browser.
    let quiet = w.start("personal", None).await;
    assert_eq!(quiet.browser_url, None);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let opened = std::fs::read_to_string(w.dir.join("opened.log")).unwrap();
    assert_eq!(opened.lines().count(), 1, "{opened}");
}

/// G6 through the new flow: the global account signs in on `~/.claude`;
/// signing in to another known account there makes that one global.
#[tokio::test]
async fn on_the_slot_a_sign_in_to_another_account_makes_it_global() {
    let w = world("slot", &["personal"]).await;
    assert_eq!(w.ctx.collector.accounts().registry().global, "default");
    let started = w.start("default", None).await;
    assert_eq!(started.email.as_deref(), Some("you@company.com"));
    // Signed in with Work again: still Work.
    let done = w
        .code(&started.login_id, "demo-ok#demo-state")
        .await
        .unwrap();
    assert_eq!(done.account.id, "default");
    assert_eq!(w.token_in(&w.paths.claude_home).as_deref(), Some("work"));
    // The person signs in to Personal on the page instead.
    let started = w.start("default", None).await;
    let done = w
        .code(&started.login_id, "demo-ok:personal#demo-state")
        .await
        .unwrap();
    assert_eq!(done.account.id, "personal");
    let registry = w.ctx.collector.accounts().registry();
    assert_eq!(registry.global, "personal");
    assert_eq!(
        registry.last_switch.unwrap().reason.as_deref(),
        Some("login")
    );
    assert_eq!(
        w.token_in(&w.paths.claude_home).as_deref(),
        Some("personal")
    );
}

/// The link works for its time only; then Claude Code is stopped.
#[tokio::test]
async fn a_link_expires() {
    let w = world("expiry", &["personal"]).await;
    w.runner(|runner| runner.ttl_ms = Some(2_000));
    let started = w.start("personal", None).await;
    let entry = eventually("expired", || async {
        w.login(&started.login_id)
            .await
            .filter(|l| l.state == LoginState::Expired)
    })
    .await;
    let error = entry.error.unwrap();
    assert_eq!(error.code, "expired");
    // The time it really had, not the usual 10 minutes.
    assert!(
        error.message.contains("after 2 seconds"),
        "{}",
        error.message
    );
    let again = w
        .code(&started.login_id, "demo-ok#demo-state")
        .await
        .unwrap_err();
    assert_eq!(again["code"], "expired");
    assert_eq!(again["message"], error.message.as_str());
}

/// An account nobody knows, or no Claude Code: one JSON error.
#[tokio::test]
async fn starting_fails_in_one_json_object() {
    let w = world("fails", &[]).await;
    let error = account_fns::login_start(
        &w.ctx,
        account_fns::LoginStartInput {
            account: "nobody".into(),
            browser: None,
        },
    )
    .await
    .unwrap_err();
    let error: serde_json::Value = serde_json::from_str(&error).unwrap();
    assert_eq!(error["code"], "failed");
    assert!(error["message"].as_str().unwrap().contains("nobody"));
    // A Claude Code that ends at once says why.
    let broken = w.dir.join("bin/broken");
    script(
        &broken,
        "#!/bin/sh\necho 'Error: something is off.' >&2\nexit 3\n",
    );
    w.ctx.collector.accounts().logins.set_runner(Runner {
        claude: Some(broken),
        ..Runner::default()
    });
    let error = account_fns::login_start(
        &w.ctx,
        account_fns::LoginStartInput {
            account: "default".into(),
            browser: None,
        },
    )
    .await
    .unwrap_err();
    let error: serde_json::Value = serde_json::from_str(&error).unwrap();
    assert_eq!(error["code"], "failed");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("Error: something is off."),
        "{error}"
    );
}

/// Two starts for one account at once (two devices): one login stays, the
/// later one; the other start answers `gone`. Neither leaves a Claude
/// Code behind.
#[tokio::test]
async fn two_starts_at_once_leave_one_login() {
    let w = world("race", &["personal"]).await;
    for _ in 0..3 {
        let start = || {
            account_fns::login_start(
                &w.ctx,
                account_fns::LoginStartInput {
                    account: "personal".into(),
                    browser: None,
                },
            )
        };
        let (a, b) = tokio::join!(start(), start());
        let won: Vec<&account_fns::LoginStartOutput> = [&a, &b]
            .into_iter()
            .filter_map(|r| r.as_ref().ok())
            .collect();
        assert_eq!(won.len(), 1, "{a:?} {b:?}");
        let lost = [&a, &b].into_iter().find_map(|r| r.as_ref().err()).unwrap();
        let lost: serde_json::Value = serde_json::from_str(lost).unwrap();
        assert_eq!(lost["code"], "gone", "{lost}");
        let live: Vec<ClaudeLogin> = w
            .logins()
            .await
            .into_iter()
            .filter(|l| l.account == "personal" && l.state == LoginState::WaitingCode)
            .collect();
        assert_eq!(live.len(), 1, "{live:?}");
        assert_eq!(live[0].login_id, won[0].login_id);
    }
    // The one left signs in.
    let id = w.logins().await.last().unwrap().login_id.clone();
    assert!(w.code(&id, "demo-ok#demo-state").await.is_ok());
}

/// Two codes at once: one is typed, the other call waits for its outcome.
#[tokio::test]
async fn two_codes_at_once_get_one_outcome() {
    let w = world("codes", &["personal"]).await;
    let started = w.start("personal", None).await;
    let (a, b) = tokio::join!(
        w.code(&started.login_id, "AbCdEf123#xyz"),
        w.code(&started.login_id, "demo-ok#demo-state")
    );
    let outcome = |r: &Result<account_fns::LoginCodeOutput, serde_json::Value>| match r {
        Ok(done) => format!("done {}", done.account.id),
        Err(error) => error["code"].as_str().unwrap().to_string(),
    };
    assert_eq!(outcome(&a), outcome(&b));
}

/// Claude Code saying it signed in proves nothing by itself: the sign-in
/// has to be in the account's folder.
#[tokio::test]
async fn a_sign_in_that_did_not_land_fails() {
    let w = world("hollow", &["personal"]).await;
    std::fs::remove_file(w.paths.account_dir("personal").join(".credentials.json")).unwrap();
    let started = w.start("personal", None).await;
    let error = w
        .code(&started.login_id, "demo-hollow#demo-state")
        .await
        .unwrap_err();
    assert_eq!(error["code"], "failed", "{error}");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("holds no sign-in"),
        "{error}"
    );
    assert_eq!(
        w.login(&started.login_id).await.unwrap().state,
        LoginState::Failed
    );
}

/// A code Claude Code takes longer to check than `login_code` waits: it is
/// not stopped mid-way, and its outcome stands.
#[tokio::test]
async fn a_slow_check_is_not_cut_off() {
    let w = world("slow", &["personal"]).await;
    std::fs::remove_file(w.paths.account_dir("personal").join(".credentials.json")).unwrap();
    w.runner(|runner| runner.code_wait = Some(Duration::from_secs(1)));
    let started = w.start("personal", None).await;
    let done = w
        .code(&started.login_id, "demo-slow#demo-state")
        .await
        .unwrap();
    assert_eq!(done.account.id, "personal");
    assert!(done.account.signed_in);
    assert_eq!(
        w.token_in(&w.paths.account_dir("personal")).as_deref(),
        Some("personal")
    );
}

/// The link running out while a code is checked: the check finishes.
#[tokio::test]
async fn a_link_expiring_mid_check_lets_the_check_finish() {
    let w = world("late", &["personal"]).await;
    std::fs::remove_file(w.paths.account_dir("personal").join(".credentials.json")).unwrap();
    w.runner(|runner| runner.ttl_ms = Some(2_000));
    let started = w.start("personal", None).await;
    let done = w
        .code(&started.login_id, "demo-slow#demo-state")
        .await
        .unwrap();
    assert_eq!(done.account.id, "personal");
    assert_eq!(
        w.login(&started.login_id).await.unwrap().state,
        LoginState::Done
    );
}

/// Cancelled while a code is checked: the login is gone at once, and
/// Claude Code still gets to save what it was saving.
#[tokio::test]
async fn cancelling_mid_check_does_not_cut_a_save_short() {
    let w = world("cancel-check", &["personal"]).await;
    std::fs::remove_file(w.paths.account_dir("personal").join(".credentials.json")).unwrap();
    let started = w.start("personal", None).await;
    let (typed, ()) = tokio::join!(w.code(&started.login_id, "demo-slow#demo-state"), async {
        eventually("checking", || async {
            w.login(&started.login_id)
                .await
                .filter(|l| l.state == LoginState::Checking)
        })
        .await;
        account_fns::login_cancel(
            &w.ctx,
            account_fns::LoginCancelInput {
                login_id: started.login_id.clone(),
            },
        )
        .await
        .unwrap();
        assert!(w.login(&started.login_id).await.is_none());
    });
    assert_eq!(typed.unwrap_err()["code"], "gone");
    eventually("the sign-in saved", || async {
        w.token_in(&w.paths.account_dir("personal"))
            .filter(|token| token == "personal")
    })
    .await;
}

/// A Claude Code that ignores SIGTERM is killed.
#[tokio::test]
async fn a_claude_code_deaf_to_sigterm_is_killed() {
    let w = world("stubborn", &["personal"]).await;
    w.runner(|runner| runner.env.push(("FAKE_CLAUDE_STUBBORN".into(), "1".into())));
    let started = w.start("personal", None).await;
    let pid = w.fake_pid();
    let alive = |pid: i32| {
        // SAFETY: signal 0 only asks whether the process exists.
        unsafe { libc::kill(pid, 0) == 0 }
    };
    assert!(alive(pid));
    account_fns::login_cancel(
        &w.ctx,
        account_fns::LoginCancelInput {
            login_id: started.login_id.clone(),
        },
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert!(alive(pid), "SIGTERM is ignored: it is still there");
    eventually("killed", || async { (!alive(pid)).then_some(()) }).await;
}
