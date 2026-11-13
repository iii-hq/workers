//! Against a real tmux on a private socket (`-L iii-tmux-test-<uuid>`), so
//! the developer's own sessions are never touched. Skips when tmux is not
//! installed. Every scenario kills its server at the end, pass or fail.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use iii_sdk::{scope, Caller, InvocationCtx};
use tmux::collector::{Collector, ConsoleClients};
use tmux::config::WorkerConfig;
use tmux::events::Monitor;
use tmux::functions::{self, panes, sessions, windows, Ctx, EmptyInput};
use tmux::model::AgentState;
use tmux::terminal::protocol::{
    OpenRequest, OutputEvent, ReplayRequest, SwitchRequest, TerminalRef, WriteRequest,
};
use tmux::terminal::{OutputSink, TerminalManager};
use tmux::tmux_cli::WindowArg;

const CALLER: &str = "worker-under-test";
const OUTPUT: &str = "iii::tmux-ui::pty-output::console-test";

struct Server {
    socket: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::process::Command::new("tmux")
            .args(["-L", &self.socket, "kill-server"])
            .env_remove("TMUX")
            .output();
        // tmux leaves the socket file behind; `-L` puts it in
        // $TMUX_TMPDIR/tmux-<uid>/ (default /tmp).
        let dir = std::env::var("TMUX_TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
        // SAFETY: getuid has no preconditions and cannot fail.
        let uid = unsafe { libc::getuid() };
        let _ = std::fs::remove_file(format!("{dir}/tmux-{uid}/{}", self.socket));
    }
}

fn tmux_installed() -> bool {
    std::process::Command::new("tmux")
        .arg("-V")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[derive(Default)]
struct Captured(Mutex<Vec<OutputEvent>>);

impl OutputSink for Captured {
    fn deliver(
        &self,
        _function_id: String,
        event: OutputEvent,
    ) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        self.0.lock().unwrap().push(event);
        Box::pin(async { true })
    }
}

struct World {
    _server: Server,
    ctx: Ctx,
    collector: Arc<Collector>,
}

fn world() -> World {
    let socket = format!("iii-tmux-test-{}", uuid::Uuid::new_v4().simple());
    let config = WorkerConfig {
        socket_name: Some(socket.clone()),
        ..WorkerConfig::default()
    }
    .into_shared();
    let collector = Arc::new(Collector::sandboxed(config, ConsoleClients::default()));
    // No test reads this computer's Claude Code sign-in or reaches the network.
    assert!(!collector.accounts().can_fetch());
    let monitor = Monitor::new(
        iii_sdk::IIIClient::new("ws://stub-not-connected:0"),
        collector.clone(),
    );
    World {
        _server: Server { socket },
        ctx: Ctx {
            collector: collector.clone(),
            monitor,
            transcripts: Default::default(),
        },
        collector,
    }
}

async fn eventually<T, F, Fut>(what: &str, mut probe: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(value) = probe().await {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn sessions_and_windows_lifecycle() {
    if !tmux_installed() {
        eprintln!("tmux not installed; skipping");
        return;
    }
    let w = world();

    let empty = functions::sessions_list(&w.ctx, EmptyInput {})
        .await
        .unwrap();
    assert!(empty.tmux.available);
    assert!(!empty.tmux.server_running);
    assert!(empty.sessions.is_empty());

    let created = sessions::create(
        &w.ctx,
        sessions::CreateSessionInput {
            name: Some("alpha one".into()),
            cwd: Some("/".into()),
            command: None,
            window_name: Some("main".into()),
            remain_on_exit: false,
            claude: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(created.session, "alpha one");

    let second = windows::create(
        &w.ctx,
        windows::CreateWindowInput {
            session: "alpha one".into(),
            name: Some("logs".into()),
            cwd: None,
            command: None,
            select: Some(false),
            remain_on_exit: false,
            claude: None,
        },
    )
    .await
    .unwrap();

    let snapshot = functions::sessions_list(&w.ctx, EmptyInput {})
        .await
        .unwrap();
    assert!(snapshot.tmux.server_running);
    let session = &snapshot.sessions[0];
    assert_eq!(session.id, created.session_id);
    assert_eq!(
        session
            .windows
            .iter()
            .map(|w| (w.name.as_str(), w.active))
            .collect::<Vec<_>>(),
        vec![("main", true), ("logs", false)],
        "select: false keeps the current window"
    );
    // Before tmux 3.2 a session's directory can't be read, and tmux starts
    // the window in the home folder.
    let version = functions::status(&w.ctx, EmptyInput {})
        .await
        .unwrap()
        .version
        .unwrap_or_default();
    let unset = if tmux::tmux_cli::version_at_least(&version, 3, 2) == Some(false) {
        std::env::var("HOME").unwrap()
    } else {
        "/".to_string()
    };
    // pane_current_path settles once the shell has started.
    eventually("both panes to report their directory", || async {
        let snapshot = functions::sessions_list(&w.ctx, EmptyInput {}).await.ok()?;
        let cwds: Vec<_> = snapshot.sessions[0]
            .windows
            .iter()
            .map(|window| window.panes[0].cwd.clone())
            .collect();
        (cwds == ["/".to_string(), unset.clone()]).then_some(())
    })
    .await;

    windows::select(
        &w.ctx,
        windows::WindowInput {
            session: created.session_id.clone(),
            window: WindowArg::Text(second.window_id.clone()),
        },
    )
    .await
    .unwrap();
    windows::rename(
        &w.ctx,
        windows::RenameWindowInput {
            session: "alpha one".into(),
            window: WindowArg::Index(second.window_index),
            name: "build".into(),
        },
    )
    .await
    .unwrap();
    let snapshot = functions::sessions_list(&w.ctx, EmptyInput {})
        .await
        .unwrap();
    let names: Vec<(&str, bool)> = snapshot.sessions[0]
        .windows
        .iter()
        .map(|w| (w.name.as_str(), w.active))
        .collect();
    assert_eq!(names, vec![("main", false), ("build", true)]);

    let missing = windows::select(
        &w.ctx,
        windows::WindowInput {
            session: "alpha".into(),
            window: WindowArg::Index(0),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(
        missing, "There is no session named \"alpha\".",
        "names never prefix-match"
    );

    sessions::rename(
        &w.ctx,
        sessions::RenameSessionInput {
            session: "alpha one".into(),
            name: "beta".into(),
        },
    )
    .await
    .unwrap();
    windows::kill(
        &w.ctx,
        windows::WindowInput {
            session: "beta".into(),
            window: WindowArg::Text(second.window_id),
        },
    )
    .await
    .unwrap();
    let snapshot = functions::sessions_list(&w.ctx, EmptyInput {})
        .await
        .unwrap();
    assert_eq!(snapshot.sessions[0].name, "beta");
    assert_eq!(snapshot.sessions[0].windows.len(), 1);

    sessions::kill(
        &w.ctx,
        sessions::SessionInput {
            session: "beta".into(),
        },
    )
    .await
    .unwrap();
    let after = functions::sessions_list(&w.ctx, EmptyInput {})
        .await
        .unwrap();
    assert!(after.sessions.is_empty());
}

#[tokio::test]
async fn names_are_taken_literally_and_never_as_ids() {
    if !tmux_installed() {
        eprintln!("tmux not installed; skipping");
        return;
    }
    let w = world();
    let raw = |args: &[&str]| {
        let output = std::process::Command::new("tmux")
            .args(["-L", &w._server.socket])
            .args(args)
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };
    // tmux expands formats in names (2.7 and later; -n from 3.1): `#` is
    // kept as typed.
    let created = sessions::create(
        &w.ctx,
        sessions::CreateSessionInput {
            name: Some("a#{session_id}b".into()),
            window_name: Some("w#{window_id}x".into()),
            cwd: Some("/".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(created.session, "a#{session_id}b");
    windows::create(
        &w.ctx,
        windows::CreateWindowInput {
            session: created.session_id.clone(),
            name: Some("n##m".into()),
            select: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    windows::rename(
        &w.ctx,
        windows::RenameWindowInput {
            session: "a#{session_id}b".into(),
            window: WindowArg::Index(created.window_index),
            name: "r#(echo hi)s".into(),
        },
    )
    .await
    .unwrap();
    sessions::rename(
        &w.ctx,
        sessions::RenameSessionInput {
            session: created.session_id.clone(),
            name: "s#{host}t".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        windows_line(&w.ctx, "s#{host}t").await.unwrap(),
        format!(
            "{}:r#(echo hi)s* {}:n##m",
            created.window_index,
            created.window_index + 1
        )
    );

    // Names shaped like ids are refused.
    let id_like =
        |kind: &str| format!("A {kind} name can't look like a tmux id such as $2, @3 or %4.");
    let refused = [
        (
            sessions::create(
                &w.ctx,
                sessions::CreateSessionInput {
                    name: Some("$9".into()),
                    ..Default::default()
                },
            )
            .await
            .map(drop),
            id_like("session"),
        ),
        (
            sessions::rename(
                &w.ctx,
                sessions::RenameSessionInput {
                    session: created.session_id.clone(),
                    name: "%3".into(),
                },
            )
            .await
            .map(drop),
            id_like("session"),
        ),
        (
            windows::create(
                &w.ctx,
                windows::CreateWindowInput {
                    session: created.session_id.clone(),
                    name: Some("@2".into()),
                    ..Default::default()
                },
            )
            .await
            .map(drop),
            id_like("window"),
        ),
    ];
    for (result, sentence) in refused {
        assert_eq!(result.unwrap_err(), sentence);
    }

    // A session named like an id, made outside this worker, is never taken
    // for the session that has that id.
    let named_like_ours = raw(&[
        "new-session",
        "-d",
        "-P",
        "-F",
        "#{session_id}",
        "-s",
        &created.session_id,
    ]);
    let select = |session: &str| {
        windows::select(
            &w.ctx,
            windows::WindowInput {
                session: session.into(),
                window: WindowArg::Index(0),
            },
        )
    };
    assert_eq!(
        select(&created.session_id).await.unwrap_err(),
        format!(
            "\"{0}\" is session {0}'s id and session {1}'s name; rename session {1} to tell them apart.",
            created.session_id, named_like_ours
        )
    );
    let stray = raw(&[
        "new-session",
        "-d",
        "-P",
        "-F",
        "#{session_id}",
        "-s",
        "$99",
    ]);
    assert_eq!(
        select("$99").await.unwrap_err(),
        format!("There is no session $99; the session named \"$99\" is {stray}.")
    );
    assert_eq!(
        select(&stray).await.unwrap().session_id,
        stray,
        "its own id still reaches it"
    );
}

/// A fresh folder under the system's temp folder, removed on drop.
struct Folder(std::path::PathBuf);

impl Folder {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("iii-tmux-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&path).unwrap();
        // tmux reports a pane's folder with links resolved: macOS's temporary
        // folder is under /var, a link to /private/var.
        Self(std::fs::canonicalize(&path).unwrap())
    }

    /// A subfolder named `name`, created.
    fn sub(&self, name: &str) -> String {
        let path = self.0.join(name);
        std::fs::create_dir_all(&path).unwrap();
        path.to_string_lossy().into_owned()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The current directory of a pane, once its shell has started.
async fn pane_cwd(ctx: &Ctx, pane_id: &str, expected: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let snapshot = functions::sessions_list(ctx, EmptyInput {}).await.unwrap();
        let cwd = snapshot
            .sessions
            .iter()
            .flat_map(|s| &s.windows)
            .flat_map(|w| &w.panes)
            .find(|p| p.id == pane_id)
            .map(|p| p.cwd.clone())
            .unwrap_or_default();
        if cwd == expected || Instant::now() > deadline {
            return cwd;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn start_directories_are_taken_literally() {
    if !tmux_installed() {
        eprintln!("tmux not installed; skipping");
        return;
    }
    let w = world();
    let folder = Folder::new();
    // tmux expands formats in -c: unescaped, these would name other folders.
    let braces = folder.sub("x#{session_name}y");
    let doubled = folder.sub("p##q");
    let session = sessions::create(
        &w.ctx,
        sessions::CreateSessionInput {
            name: Some("literal".into()),
            cwd: Some(braces.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(pane_cwd(&w.ctx, &session.pane_id, &braces).await, braces);
    let window = windows::create(
        &w.ctx,
        windows::CreateWindowInput {
            session: "literal".into(),
            name: None,
            cwd: Some(doubled.clone()),
            command: None,
            select: Some(false),
            remain_on_exit: false,
            claude: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(pane_cwd(&w.ctx, &window.pane_id, &doubled).await, doubled);
}

/// A session's windows as `index:name`, `*` on the current one; `None` once
/// the session is gone.
async fn windows_line(ctx: &Ctx, session: &str) -> Option<String> {
    let snapshot = functions::sessions_list(ctx, EmptyInput {}).await.unwrap();
    let session = snapshot.sessions.iter().find(|s| s.name == session)?;
    Some(
        session
            .windows
            .iter()
            .map(|w| format!("{}:{}{}", w.index, w.name, if w.active { "*" } else { "" }))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// `sessions::create` and `windows::create` for the move tests.
struct Builder<'a>(&'a Ctx);

impl Builder<'_> {
    async fn session(&self, name: &str, window: &str) -> sessions::CreatedSession {
        sessions::create(
            self.0,
            sessions::CreateSessionInput {
                name: Some(name.into()),
                cwd: Some("/".into()),
                window_name: Some(window.into()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }

    async fn window(&self, session: &str, name: &str) -> windows::CreatedWindow {
        windows::create(
            self.0,
            windows::CreateWindowInput {
                session: session.into(),
                name: Some(name.into()),
                select: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }

    async fn kill_window(&self, session: &str, window: &str) {
        windows::kill(
            self.0,
            windows::WindowInput {
                session: session.into(),
                window: WindowArg::Text(window.into()),
            },
        )
        .await
        .unwrap();
    }

    async fn move_to(
        &self,
        window: &str,
        from: &str,
        to: &str,
        before: Option<&str>,
        after: Option<&str>,
    ) -> Result<windows::MovedWindow, String> {
        let arg = |window: &str| WindowArg::Text(window.into());
        windows::move_window(
            self.0,
            windows::MoveWindowInput {
                session: from.into(),
                window: arg(window),
                target_session: to.into(),
                before: before.map(arg),
                after: after.map(arg),
            },
        )
        .await
    }
}

#[tokio::test]
async fn windows_move_to_another_session() {
    if !tmux_installed() {
        eprintln!("tmux not installed; skipping");
        return;
    }
    let w = world();
    let b = Builder(&w.ctx);
    let src = b.session("src", "a0").await;
    let a1 = b.window("src", "a1").await;
    let a2 = b.window("src", "a2").await;
    let dst = b.session("dst", "b0").await;
    let b1 = b.window("dst", "b1").await;
    b.window("dst", "b2").await;
    b.kill_window("dst", &b1.window_id).await;
    assert_eq!(windows_line(&w.ctx, "dst").await.unwrap(), "0:b0* 2:b2");

    // A plain move goes after the last window, not into the gap at 1, and
    // leaves the destination's current window as it was.
    let moved = b
        .move_to(&a1.window_id, "src", &dst.session_id, None, None)
        .await
        .unwrap();
    assert_eq!(
        (
            moved.session_id.as_str(),
            moved.window_id.as_str(),
            moved.window_index,
            moved.source_session_closed
        ),
        (dst.session_id.as_str(), a1.window_id.as_str(), 3, false)
    );
    assert_eq!(
        windows_line(&w.ctx, "dst").await.unwrap(),
        "0:b0* 2:b2 3:a1"
    );
    assert_eq!(windows_line(&w.ctx, "src").await.unwrap(), "0:a0* 2:a2");

    // Refusals change nothing.
    let refusals = [
        (
            b.move_to("2", "dst", "dst", None, None).await,
            "The window is already in that session.".to_string(),
        ),
        (
            b.move_to("0", "src", "nope", None, None).await,
            "There is no session named \"nope\".".to_string(),
        ),
        (
            b.move_to("0", "gone", "dst", None, None).await,
            "There is no session named \"gone\".".to_string(),
        ),
        (
            b.move_to("0", "src", "dst", Some("0"), Some("2")).await,
            "Give before or after, not both.".to_string(),
        ),
        (
            b.move_to("7", "src", "dst", None, None).await,
            format!("Session {} has no window at index 7.", src.session_id),
        ),
    ];
    for (result, expected) in refusals {
        assert_eq!(result.unwrap_err(), expected);
    }
    assert_eq!(
        windows_line(&w.ctx, "dst").await.unwrap(),
        "0:b0* 2:b2 3:a1"
    );
    assert_eq!(windows_line(&w.ctx, "src").await.unwrap(), "0:a0* 2:a2");

    // The last window of a session: tmux closes the session, which the
    // answer says; the window and its pane carry on in the other one.
    let moved = b.move_to(&a2.window_id, "src", "dst", None, None).await;
    assert_eq!(moved.unwrap().window_index, 4);
    let moved = b
        .move_to(&src.window_id, &src.session_id, "dst", None, None)
        .await
        .unwrap();
    assert_eq!(
        (
            moved.window_id.as_str(),
            moved.window_index,
            moved.source_session_closed
        ),
        (src.window_id.as_str(), 5, true)
    );
    assert_eq!(windows_line(&w.ctx, "src").await, None);
    assert_eq!(
        windows_line(&w.ctx, "dst").await.unwrap(),
        "0:b0* 2:b2 3:a1 4:a2 5:a0"
    );
    let snapshot = functions::sessions_list(&w.ctx, EmptyInput {})
        .await
        .unwrap();
    assert_eq!(snapshot.sessions.len(), 1);
    let carried = snapshot.sessions[0]
        .windows
        .iter()
        .find(|window| window.id == src.window_id)
        .unwrap();
    assert_eq!(carried.panes[0].id, src.pane_id);
}

#[tokio::test]
async fn windows_move_before_and_after_another() {
    if !tmux_installed() {
        eprintln!("tmux not installed; skipping");
        return;
    }
    let w = world();
    let b = Builder(&w.ctx);
    let src = b.session("src", "a0").await;
    let a1 = b.window("src", "a1").await;
    let dst = b.session("dst", "b0").await;
    let b1 = b.window("dst", "b1").await;
    b.window("dst", "b2").await;
    let b3 = b.window("dst", "b3").await;
    let b4 = b.window("dst", "b4").await;
    b.kill_window("dst", &b3.window_id).await;
    assert_eq!(
        windows_line(&w.ctx, "dst").await.unwrap(),
        "0:b0* 1:b1 2:b2 4:b4"
    );

    // Before tmux 3.2, -a means "after the current window" whatever the
    // target, and a move inside one session with it ends tmux 2.6's server:
    // both sides are refused in one sentence, and nothing moves.
    let version = functions::status(&w.ctx, EmptyInput {})
        .await
        .unwrap()
        .version
        .unwrap();
    if tmux::tmux_cli::version_at_least(&version, 3, 2) == Some(false) {
        for (before, after, word) in [(Some("1"), None, "before"), (None, Some("1"), "after")] {
            assert_eq!(
                b.move_to(&a1.window_id, "src", "dst", before, after)
                    .await
                    .unwrap_err(),
                format!(
                    "Moving a window {word} another needs tmux 3.2 or later, and this computer has tmux {version}."
                )
            );
            assert_eq!(
                b.move_to("0", "dst", "dst", before, after)
                    .await
                    .unwrap_err(),
                format!(
                    "Moving a window {word} another needs tmux 3.2 or later, and this computer has tmux {version}."
                )
            );
        }
        assert_eq!(
            windows_line(&w.ctx, "dst").await.unwrap(),
            "0:b0* 1:b1 2:b2 4:b4"
        );
        assert_eq!(windows_line(&w.ctx, "src").await.unwrap(), "0:a0* 1:a1");
        return;
    }

    // `after` takes the index right after its neighbour; the windows from
    // there up to the first free index move up one (b4 stays at 4).
    let moved = b
        .move_to(&a1.window_id, "src", "dst", None, Some("0"))
        .await
        .unwrap();
    assert_eq!(
        (moved.window_index, moved.source_session_closed),
        (1, false)
    );
    assert_eq!(
        windows_line(&w.ctx, "dst").await.unwrap(),
        "0:b0* 1:a1 2:b1 3:b2 4:b4"
    );

    // `before`, and the source's last window: its session closes.
    let moved = b
        .move_to("0", "src", "dst", Some(&b4.window_id), None)
        .await
        .unwrap();
    assert_eq!((moved.window_index, moved.source_session_closed), (4, true));
    assert_eq!(
        windows_line(&w.ctx, "dst").await.unwrap(),
        "0:b0* 1:a1 2:b1 3:b2 4:a0 5:b4"
    );

    // Reordering the current window inside its session keeps it current.
    let moved = b
        .move_to("0", "dst", "dst", None, Some(&b4.window_id))
        .await
        .unwrap();
    assert_eq!(
        (moved.window_index, moved.source_session_closed),
        (6, false)
    );
    assert_eq!(
        windows_line(&w.ctx, "dst").await.unwrap(),
        "1:a1 2:b1 3:b2 4:a0 5:b4 6:b0*"
    );

    // Refusals change nothing.
    assert_eq!(
        b.move_to("1", "dst", "dst", None, Some("1"))
            .await
            .unwrap_err(),
        "A window can't be moved before or after itself."
    );
    assert_eq!(
        b.move_to("1", "dst", "dst", None, Some("@999"))
            .await
            .unwrap_err(),
        format!("Session {} has no window @999.", dst.session_id)
    );
    assert_eq!(
        windows_line(&w.ctx, "dst").await.unwrap(),
        "1:a1 2:b1 3:b2 4:a0 5:b4 6:b0*"
    );

    // `before` inside the session shifts from its neighbour up to the gap
    // the moved window leaves behind.
    let moved = b
        .move_to(&b4.window_id, "dst", "dst", Some(&a1.window_id), None)
        .await
        .unwrap();
    assert_eq!(moved.window_index, 1);
    assert_eq!(
        windows_line(&w.ctx, "dst").await.unwrap(),
        "1:b4 2:a1 3:b1 4:b2 5:a0 7:b0*"
    );
    let _ = (src, b1);
}

#[tokio::test]
async fn agents_and_ports_are_found_in_their_panes() {
    if !tmux_installed() || which("bash").is_none() || which("python3").is_none() {
        eprintln!("tmux, bash or python3 missing; skipping");
        return;
    }
    let w = world();
    // A process whose argv[0] is `claude`, showing Claude Code's working line.
    let agent = sessions::create(
        &w.ctx,
        sessions::CreateSessionInput {
            name: Some("agent".into()),
            cwd: Some("/".into()),
            command: Some(
                "bash -c \"printf '✻ Thinking…\\n  esc to interrupt\\n'; exec -a claude sleep 60\""
                    .into(),
            ),
            window_name: None,
            remain_on_exit: false,
            claude: None,
        },
    )
    .await
    .unwrap();
    // A server listening on an ephemeral port.
    let server = sessions::create(
        &w.ctx,
        sessions::CreateSessionInput {
            name: Some("server".into()),
            cwd: Some("/".into()),
            command: Some(
                "python3 -c \"import socket,time; s=socket.socket(); s.bind(('127.0.0.1',0)); s.listen(); print(s.getsockname()[1], flush=True); time.sleep(60)\"".into(),
            ),
            window_name: None,
            remain_on_exit: false,
            claude: None,
        },
    )
    .await
    .unwrap();

    let status = eventually("the claude agent to be detected", || async {
        let snapshot = functions::sessions_list(&w.ctx, EmptyInput {}).await.ok()?;
        let pane = snapshot
            .sessions
            .iter()
            .find(|s| s.id == agent.session_id)?
            .windows[0]
            .panes[0]
            .clone();
        pane.agent
    })
    .await;
    assert_eq!(status.id, "claude");
    assert_eq!(status.state, AgentState::Working);
    assert_eq!(
        status.title, None,
        "tmux's default pane title is the host name"
    );

    let listener = eventually("the python listener to be located", || async {
        let ports = functions::ports_list(&w.ctx, EmptyInput {}).await.ok()?;
        ports.listeners.into_iter().find(|l| {
            l.location
                .as_ref()
                .is_some_and(|loc| loc.pane_id == server.pane_id)
        })
    })
    .await;
    assert_eq!(listener.address, "127.0.0.1");
    let overview = w.collector.overview().await;
    let pane = &overview
        .sessions
        .sessions
        .iter()
        .find(|s| s.id == server.session_id)
        .unwrap()
        .windows[0]
        .panes[0];
    assert!(pane.ports.iter().any(|p| p.port == listener.port));
}

#[tokio::test]
async fn console_terminal_is_a_live_tmux_client() {
    if !tmux_installed() {
        eprintln!("tmux not installed; skipping");
        return;
    }
    let w = world();
    let first = sessions::create(
        &w.ctx,
        sessions::CreateSessionInput {
            name: Some("one".into()),
            cwd: Some("/".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let second = sessions::create(
        &w.ctx,
        sessions::CreateSessionInput {
            name: Some("two".into()),
            cwd: Some("/".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let sink = Arc::new(Captured::default());
    let console = ConsoleClients::default();
    let collector = Arc::new(Collector::sandboxed(w.collector_config(), console.clone()));
    // As on a 0.24 engine, which identifies the caller only in the payload.
    let manager = TerminalManager::new(
        sink.clone(),
        collector.clone(),
        console,
        Arc::new(|| {}),
        Arc::new(|| Some(false)),
    );

    let refused = manager
        .open(OpenRequest {
            session: "one".into(),
            window: None,
            cols: 100,
            rows: 30,
            output_function_id: "iii::shell-ui::pty-output::console-x".into(),
            caller_worker_id: Some(CALLER.into()),
        })
        .await
        .unwrap_err();
    assert!(refused.contains("not a tmux console handler"));

    let opened = manager
        .open(OpenRequest {
            session: "one".into(),
            window: None,
            cols: 100,
            rows: 30,
            output_function_id: OUTPUT.into(),
            caller_worker_id: Some(CALLER.into()),
        })
        .await
        .unwrap();
    assert_eq!(opened.session_id, first.session_id);
    let reference = TerminalRef {
        terminal_id: opened.terminal_id.clone(),
        access_key: opened.access_key.clone(),
        caller_worker_id: Some(CALLER.into()),
    };

    // The client shows up as attached, and as a console client.
    let attached = eventually("the console client to attach", || async {
        let snapshot = collector.overview().await.sessions;
        let one = snapshot
            .sessions
            .iter()
            .find(|s| s.id == first.session_id)?
            .clone();
        (one.attached == 1 && one.console_attached == 1).then_some(one)
    })
    .await;
    assert_eq!(attached.name, "one");

    // Keystrokes reach the pane and its output comes back as frames.
    manager
        .write(WriteRequest {
            terminal: reference.clone(),
            data: BASE64.encode("echo hi-from-$((40+2))\r"),
        })
        .await
        .unwrap();
    let output = || async {
        let replay = manager
            .replay(ReplayRequest {
                terminal: reference.clone(),
                after_sequence: 0,
            })
            .await
            .ok()?;
        let text: Vec<u8> = replay
            .frames
            .iter()
            .flat_map(|frame| BASE64.decode(&frame.data).unwrap())
            .collect();
        Some(String::from_utf8_lossy(&text).into_owned())
    };
    eventually("the echoed output", || async {
        output().await?.contains("hi-from-42").then_some(())
    })
    .await;

    // An OSC 8 link a program prints reaches the page as OSC 8, not only as
    // its text: tmux keeps it for a client that declares `hyperlinks`.
    manager
        .write(WriteRequest {
            terminal: reference.clone(),
            data: BASE64.encode(
                "printf '\\033]8;;https://example.com/pr\\033\\\\pr-text\\033]8;;\\033\\\\\\n'\r",
            ),
        })
        .await
        .unwrap();
    eventually("the OSC 8 link", || async {
        // The typed command echoes `\033`, not ESC: only the link has ESC.
        // tmux names the link (`id=tmux1`) and closes it where it likes.
        output()
            .await?
            .contains(";https://example.com/pr\x1b\\pr-text")
            .then_some(())
    })
    .await;
    assert!(sink
        .0
        .lock()
        .unwrap()
        .iter()
        .all(|event| event.terminal_id == opened.terminal_id));

    // Another caller cannot drive it.
    let foreign = manager
        .write(WriteRequest {
            terminal: TerminalRef {
                caller_worker_id: Some("someone-else".into()),
                ..reference.clone()
            },
            data: BASE64.encode("x"),
        })
        .await
        .unwrap_err();
    assert!(foreign.contains("credentials are invalid"));

    // On 0.25 engines the invocation's caller is the identity; the payload
    // stamp only stands in without one.
    let invoked_by = |worker_id: &str| InvocationCtx {
        function_id: "tmux::terminal::replay".into(),
        invocation_id: "inv-1".into(),
        caller: Some(Caller {
            worker_id: Some(worker_id.into()),
            ..Caller::default()
        }),
        activation: None,
        observe_depth: 0,
        metadata: None,
        traceparent: None,
        baggage: None,
    };
    let replay_as = |terminal: TerminalRef| ReplayRequest {
        terminal,
        after_sequence: 0,
    };
    let unstamped = TerminalRef {
        caller_worker_id: None,
        ..reference.clone()
    };
    scope(invoked_by(CALLER), manager.replay(replay_as(unstamped)))
        .await
        .unwrap();
    let foreign = scope(
        invoked_by("someone-else"),
        manager.replay(replay_as(reference.clone())),
    )
    .await
    .unwrap_err();
    assert!(foreign.contains("credentials are invalid"));

    // switch moves only this client.
    manager
        .switch(SwitchRequest {
            terminal: reference.clone(),
            session: "two".into(),
            window: None,
        })
        .await
        .unwrap();
    eventually("the client to move to session two", || async {
        let snapshot = collector.overview().await.sessions;
        let two = snapshot
            .sessions
            .iter()
            .find(|s| s.id == second.session_id)?;
        (two.console_attached == 1).then_some(())
    })
    .await;

    // Closing detaches; the sessions survive.
    let closed = manager.close(reference.clone()).await.unwrap();
    assert!(closed.closed);
    eventually("the eof event", || async {
        sink.0
            .lock()
            .unwrap()
            .iter()
            .any(|event| event.eof)
            .then_some(())
    })
    .await;
    let snapshot = collector.overview().await.sessions;
    assert_eq!(snapshot.sessions.len(), 2);
    assert!(snapshot.sessions.iter().all(|s| s.attached == 0));
    assert!(manager
        .write(WriteRequest {
            terminal: reference,
            data: BASE64.encode("x")
        })
        .await
        .is_err());

    // A terminal nobody heartbeats is detached by the reaper.
    let idle = manager
        .open(OpenRequest {
            session: "two".into(),
            window: None,
            cols: 80,
            rows: 24,
            output_function_id: OUTPUT.into(),
            caller_worker_id: Some(CALLER.into()),
        })
        .await
        .unwrap();
    assert_eq!(manager.count().await, 1);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(manager.reap_idle(Duration::from_millis(10)).await, 1);
    assert_eq!(manager.count().await, 0);
    let _ = idle;
}

#[tokio::test]
async fn panes_zoom_and_capture() {
    if !tmux_installed() || which("bash").is_none() {
        eprintln!("tmux or bash missing; skipping");
        return;
    }
    let w = world();
    let created = sessions::create(
        &w.ctx,
        sessions::CreateSessionInput {
            name: Some("panes".into()),
            cwd: Some("/".into()),
            command: Some("bash -c \"printf '\\033[1mhello\\033[0m\\n'; exec sleep 60\"".into()),
            window_name: None,
            remain_on_exit: false,
            claude: None,
        },
    )
    .await
    .unwrap();
    // A window with one pane can't zoom: the answer says zoomed false (the
    // description's sentence), whichever way it is asked, and no error.
    for zoom in [Some(true), None, Some(false), Some(true)] {
        let single = panes::select(
            &w.ctx,
            panes::SelectPaneInput {
                pane: created.pane_id.clone(),
                zoom,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            (single.pane_id.as_str(), single.zoomed),
            (created.pane_id.as_str(), false)
        );
    }
    let split = std::process::Command::new("tmux")
        .args([
            "-L",
            &w._server.socket,
            "split-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
        ])
        .args(["-t", created.pane_id.as_str(), "sleep 60"])
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert!(
        split.status.success(),
        "{}",
        String::from_utf8_lossy(&split.stderr)
    );
    let second = String::from_utf8_lossy(&split.stdout).trim().to_string();

    let select = |pane: &str, zoom: Option<bool>| {
        panes::select(
            &w.ctx,
            panes::SelectPaneInput {
                pane: pane.to_string(),
                zoom,
            },
        )
    };
    let zoomed = select(&second, Some(true)).await.unwrap();
    assert_eq!(
        (zoomed.pane_id.as_str(), zoomed.zoomed),
        (second.as_str(), true)
    );
    assert_eq!(zoomed.window_id, created.window_id);
    // Without `zoom`, the zoom follows the newly selected pane.
    let moved = select(&created.pane_id, None).await.unwrap();
    assert_eq!(
        (moved.pane_id.as_str(), moved.zoomed),
        (created.pane_id.as_str(), true)
    );
    let again = select(&created.pane_id, Some(true)).await.unwrap();
    assert!(again.zoomed, "zooming the zoomed pane keeps it zoomed");
    let plain = select(&second, Some(false)).await.unwrap();
    assert_eq!(
        (plain.pane_id.as_str(), plain.zoomed),
        (second.as_str(), false)
    );
    let snapshot = functions::sessions_list(&w.ctx, EmptyInput {})
        .await
        .unwrap();
    let window = &snapshot.sessions[0].windows[0];
    assert!(!window.zoomed);
    assert!(window.panes.iter().any(|p| p.id == second && p.active));

    let capture = eventually("the first pane to print", || async {
        let capture = panes::capture(
            &w.ctx,
            panes::PaneInput {
                pane: created.pane_id.clone(),
            },
        )
        .await
        .ok()?;
        capture.text.contains("hello").then_some(capture)
    })
    .await;
    assert!(
        capture.text.contains("\u{1b}[1mhello"),
        "escapes kept: {:?}",
        capture.text
    );
    assert!(capture.width > 0 && capture.height > 0);
    assert!(!capture.alternate);

    let missing = panes::capture(
        &w.ctx,
        panes::PaneInput {
            pane: "%999".into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(missing, "There is no pane %999.");
    let invalid = select("@1", Some(true)).await.unwrap_err();
    assert!(invalid.contains("pane must be an id"), "{invalid}");
}

#[tokio::test]
async fn uploads_are_saved_for_the_terminal_that_asks() {
    use std::os::unix::fs::PermissionsExt;
    use tmux::terminal::protocol::UploadRequest;

    if !tmux_installed() {
        eprintln!("tmux not installed; skipping");
        return;
    }
    let w = world();
    sessions::create(
        &w.ctx,
        sessions::CreateSessionInput {
            name: Some("paste".into()),
            cwd: Some("/".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let manager_at = |cache: &std::path::Path| {
        let console = ConsoleClients::default();
        TerminalManager::new(
            Arc::new(Captured::default()),
            Arc::new(Collector::sandboxed(w.collector_config(), console.clone())),
            console,
            Arc::new(|| {}),
            Arc::new(|| Some(false)),
        )
        .with_upload_cache(cache.to_path_buf())
    };
    let open = |manager: &TerminalManager| {
        let manager = manager.clone();
        async move {
            let opened = manager
                .open(OpenRequest {
                    session: "paste".into(),
                    window: None,
                    cols: 80,
                    rows: 24,
                    output_function_id: OUTPUT.into(),
                    caller_worker_id: Some(CALLER.into()),
                })
                .await
                .unwrap();
            TerminalRef {
                terminal_id: opened.terminal_id,
                access_key: opened.access_key,
                caller_worker_id: Some(CALLER.into()),
            }
        }
    };
    let folder = Folder::new();
    let uploads = folder.0.join("iii/tmux/uploads");
    let manager = manager_at(&folder.0);
    let reference = open(&manager).await;
    let upload = |terminal: TerminalRef, name: &str, mime: Option<&str>, data: &[u8]| {
        manager.upload(UploadRequest {
            terminal,
            name: name.into(),
            data: BASE64.encode(data),
            mime: mime.map(str::to_string),
        })
    };
    let jpeg = b"\xff\xd8\xff\xe0\0\x10JFIF\0\x01";
    let mode = |path: &std::path::Path| path.metadata().unwrap().permissions().mode() & 0o777;

    // Any file, each in a folder of its own under its own name made safe.
    let long = format!("{}.txt", "l".repeat(296));
    // name, mime, data, the name it is kept under, the type answered
    type Case<'a> = (&'a str, Option<&'a str>, &'a [u8], &'a str, &'a str);
    let cases: [Case; 7] = [
        (
            "Quarterly report final.pdf",
            Some("application/pdf"),
            b"%PDF-1.7",
            "Quarterly-report-final.pdf",
            "application/pdf",
        ),
        (
            "relatório ação.md",
            Some("text/markdown"),
            b"# oi",
            "relat-rio-a-o.md",
            "text/markdown",
        ),
        ("../x", None, b"x", "x", "application/octet-stream"),
        (".hidden", None, b"h", "hidden", "application/octet-stream"),
        (
            &long,
            Some("text/plain"),
            b"long",
            &format!("{}.txt", "l".repeat(96)),
            "text/plain",
        ),
        (
            "IMG_0420.HEIC",
            Some("image/heic"),
            jpeg,
            "IMG_0420.jpg",
            "image/jpeg",
        ),
        ("", None, b"", "file", "application/octet-stream"),
    ];
    let mut folders = std::collections::HashSet::new();
    for (name, mime, data, kept, answered) in cases {
        let saved = upload(reference.clone(), name, mime, data).await.unwrap();
        let path = std::path::Path::new(&saved.path);
        assert!(path.is_absolute() && !saved.path.contains(' '), "{name:?}");
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            kept,
            "{name:?}"
        );
        assert_eq!((saved.bytes, saved.mime.as_str()), (data.len(), answered));
        assert_eq!(std::fs::read(path).unwrap(), data);
        let own = path.parent().unwrap();
        assert_eq!(own.parent(), Some(uploads.as_path()));
        assert_eq!(
            (mode(path), mode(own), mode(&uploads)),
            (0o600, 0o700, 0o700)
        );
        assert!(folders.insert(own.to_path_buf()), "a folder per upload");
    }
    let count = || std::fs::read_dir(&uploads).unwrap().count();
    assert_eq!(count(), 7);

    // Only the page that opened the terminal, with its key.
    let wrong_key = TerminalRef {
        access_key: "nope".into(),
        ..reference.clone()
    };
    let foreign = TerminalRef {
        caller_worker_id: Some("someone-else".into()),
        ..reference.clone()
    };
    for terminal in [wrong_key, foreign] {
        let refused = upload(terminal, "a.txt", None, b"a").await.unwrap_err();
        assert_eq!(refused, "terminal credentials are invalid");
    }
    assert_eq!(
        upload(
            reference.clone(),
            "a.txt",
            None,
            &vec![0u8; 12 * 1024 * 1024 + 1]
        )
        .await
        .unwrap_err(),
        "The file is larger than 12 MiB."
    );
    assert_eq!(count(), 7, "a refusal writes nothing");

    // An uploads folder that is a symbolic link is refused, in one sentence
    // that names no path, and nothing lands where it points.
    let elsewhere = Folder::new();
    let linked_cache = Folder::new();
    std::fs::create_dir_all(linked_cache.0.join("iii/tmux")).unwrap();
    std::os::unix::fs::symlink(&elsewhere.0, linked_cache.0.join("iii/tmux/uploads")).unwrap();
    let linked = manager_at(&linked_cache.0);
    let there = open(&linked).await;
    let refused = linked
        .upload(UploadRequest {
            terminal: there.clone(),
            name: "a.txt".into(),
            data: BASE64.encode(b"a"),
            mime: None,
        })
        .await
        .unwrap_err();
    assert_eq!(
        refused,
        "The folder for uploaded files is a symbolic link, so no file is kept there."
    );
    assert_eq!(std::fs::read_dir(&elsewhere.0).unwrap().count(), 0);
    linked.close(there).await.unwrap();

    manager.close(reference.clone()).await.unwrap();
    assert!(upload(reference, "a.txt", None, b"a")
        .await
        .unwrap_err()
        .contains("not found"));
}

#[tokio::test]
async fn panes_split_kill_and_respawn() {
    if !tmux_installed() || which("sh").is_none() {
        eprintln!("tmux or sh missing; skipping");
        return;
    }
    let w = world();
    let folder = Folder::new();
    let away = folder.sub("away #{pane_id}");
    let raw = |args: &[&str]| {
        let output = std::process::Command::new("tmux")
            .args(["-L", &w._server.socket])
            .args(args)
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };
    let current = sessions::create(
        &w.ctx,
        sessions::CreateSessionInput {
            name: Some("panes".into()),
            cwd: Some("/".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    // Not the session's current window: its pane's own folder must be used.
    let other = windows::create(
        &w.ctx,
        windows::CreateWindowInput {
            session: "panes".into(),
            name: Some("other".into()),
            cwd: Some(away.clone()),
            command: None,
            select: Some(false),
            remain_on_exit: false,
            claude: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(pane_cwd(&w.ctx, &other.pane_id, &away).await, away);

    let split = |pane: &str, direction, cwd: Option<&str>, command: Option<&str>, select| {
        panes::split(
            &w.ctx,
            panes::SplitPaneInput {
                pane: pane.into(),
                direction,
                cwd: cwd.map(str::to_string),
                command: command.map(str::to_string),
                select,
                remain_on_exit: false,
            },
        )
    };
    let right = split(
        &other.pane_id,
        panes::SplitDirection::Right,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        (right.session_id.as_str(), right.window_id.as_str()),
        (current.session_id.as_str(), other.window_id.as_str())
    );
    assert_eq!(pane_cwd(&w.ctx, &right.pane_id, &away).await, away);
    let below = split(
        &right.pane_id,
        panes::SplitDirection::Below,
        Some("/"),
        Some("sleep 60"),
        Some(false),
    )
    .await
    .unwrap();
    let snapshot = functions::sessions_list(&w.ctx, EmptyInput {})
        .await
        .unwrap();
    let window = snapshot.sessions[0]
        .windows
        .iter()
        .find(|window| window.id == other.window_id)
        .unwrap();
    let layout: Vec<(&str, bool)> = window
        .panes
        .iter()
        .map(|p| (p.id.as_str(), p.active))
        .collect();
    assert_eq!(
        layout,
        vec![
            (other.pane_id.as_str(), false),
            (right.pane_id.as_str(), true),
            (below.pane_id.as_str(), false)
        ],
        "a split is active unless select: false"
    );
    assert_eq!(
        raw(&[
            "display-message",
            "-p",
            "-t",
            &right.pane_id,
            "#{pane_at_right}#{pane_at_left}"
        ]),
        "10",
        "right means to the right"
    );
    let missing = split("%999", panes::SplitDirection::Right, None, None, None)
        .await
        .unwrap_err();
    assert_eq!(missing, "There is no pane %999.");
    let bad = split(
        &right.pane_id,
        panes::SplitDirection::Right,
        Some("rel"),
        None,
        None,
    )
    .await
    .unwrap_err();
    assert!(bad.contains("absolute"), "{bad}");

    // Splitting until there's no room ends in one sentence.
    let mut refusal = None;
    for _ in 0..40 {
        if let Err(error) = split(
            &below.pane_id,
            panes::SplitDirection::Below,
            None,
            None,
            None,
        )
        .await
        {
            refusal = Some(error);
            break;
        }
    }
    assert_eq!(refusal.as_deref(), Some("The pane is too small to split."));

    // A running program is only restarted with kill.
    let respawn = |pane: &str, command: Option<&str>, kill| {
        panes::respawn(
            &w.ctx,
            panes::RespawnPaneInput {
                pane: pane.into(),
                command: command.map(str::to_string),
                cwd: None,
                kill,
            },
        )
    };
    let pid = |pane: &str| raw(&["display-message", "-p", "-t", pane, "#{pane_pid}"]);
    let before = pid(&below.pane_id);
    assert_eq!(
        respawn(&below.pane_id, None, None).await.unwrap_err(),
        "The pane's program is still running; pass kill to restart it."
    );
    assert_eq!(pid(&below.pane_id), before, "refused, nothing changed");
    let restarted = respawn(&below.pane_id, None, Some(true)).await.unwrap();
    assert_eq!(restarted.pane_id, below.pane_id);
    assert_ne!(pid(&below.pane_id), before);
    // The command it was started with, once the new process has exec'd.
    eventually("the pane to run sleep again", || async {
        (raw(&[
            "display-message",
            "-p",
            "-t",
            &below.pane_id,
            "#{pane_current_command}",
        ]) == "sleep")
            .then_some(())
    })
    .await;
    // A dead pane restarting without kill: see
    // panes_are_kept_once_their_program_ends_only_when_asked.
    assert_eq!(
        respawn("%999", None, None).await.unwrap_err(),
        "There is no pane %999."
    );

    // A window with one pane can't zoom; the answer says so.
    let single = panes::select(
        &w.ctx,
        panes::SelectPaneInput {
            pane: current.pane_id.clone(),
            zoom: Some(true),
        },
    )
    .await
    .unwrap();
    assert!(!single.zoomed);

    // Killing: a pane, then a window's last pane, then the session's last.
    let kill = |pane: &str| {
        panes::kill(
            &w.ctx,
            panes::PaneInput {
                pane: pane.to_string(),
            },
        )
    };
    let windows_before = functions::sessions_list(&w.ctx, EmptyInput {})
        .await
        .unwrap()
        .sessions[0]
        .windows
        .iter()
        .find(|window| window.id == other.window_id)
        .unwrap()
        .panes
        .len();
    for pane in functions::sessions_list(&w.ctx, EmptyInput {})
        .await
        .unwrap()
        .sessions[0]
        .windows
        .iter()
        .find(|window| window.id == other.window_id)
        .unwrap()
        .panes
        .iter()
        .skip(1)
    {
        let killed = kill(&pane.id).await.unwrap();
        assert_eq!(
            (killed.killed, killed.window_closed, killed.session_closed),
            (true, false, false)
        );
    }
    assert!(windows_before > 2);
    let killed = kill(&other.pane_id).await.unwrap();
    assert_eq!((killed.window_closed, killed.session_closed), (true, false));
    assert_eq!(
        kill(&other.pane_id).await.unwrap_err(),
        format!("There is no pane {}.", other.pane_id)
    );
    let killed = kill(&current.pane_id).await.unwrap();
    assert_eq!((killed.window_closed, killed.session_closed), (true, true));
    let after = functions::sessions_list(&w.ctx, EmptyInput {})
        .await
        .unwrap();
    assert!(after.sessions.is_empty());
}

#[tokio::test]
async fn panes_are_kept_once_their_program_ends_only_when_asked() {
    if !tmux_installed() || which("sh").is_none() {
        eprintln!("tmux or sh missing; skipping");
        return;
    }
    let w = world();
    let raw = |args: &[&str]| {
        let output = std::process::Command::new("tmux")
            .args(["-L", &w._server.socket])
            .args(args)
            .env_remove("TMUX")
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };
    let field = |pane: &str, format: &str| raw(&["display-message", "-p", "-t", pane, format]);
    let exists = |pane: String| {
        let ctx = &w.ctx;
        async move {
            functions::sessions_list(ctx, EmptyInput {})
                .await
                .unwrap()
                .sessions
                .iter()
                .flat_map(|s| &s.windows)
                .flat_map(|w| &w.panes)
                .any(|p| p.id == pane)
        }
    };
    let anchor = sessions::create(
        &w.ctx,
        sessions::CreateSessionInput {
            name: Some("keep".into()),
            cwd: Some("/".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let session = |name: &str, command: &str, remain_on_exit| {
        sessions::create(
            &w.ctx,
            sessions::CreateSessionInput {
                name: Some(name.into()),
                cwd: Some("/".into()),
                command: Some(command.into()),
                remain_on_exit,
                ..Default::default()
            },
        )
    };
    let window = |command: &str, remain_on_exit| {
        windows::create(
            &w.ctx,
            windows::CreateWindowInput {
                session: "keep".into(),
                command: Some(command.into()),
                select: Some(false),
                remain_on_exit,
                ..Default::default()
            },
        )
    };
    let split = |command: &str, remain_on_exit| {
        panes::split(
            &w.ctx,
            panes::SplitPaneInput {
                pane: anchor.pane_id.clone(),
                direction: panes::SplitDirection::Right,
                cwd: None,
                command: Some(command.into()),
                select: Some(false),
                remain_on_exit,
            },
        )
    };

    // By default tmux closes a pane whose program ends, with its window and
    // session when it was their last.
    let ends = "sh -c 'exit 3'";
    let gone = [
        session("ends", ends, false).await.unwrap().pane_id,
        window(ends, false).await.unwrap().pane_id,
        split(ends, false).await.unwrap().pane_id,
    ];
    for pane in gone {
        eventually("the pane to close", || {
            let pane = pane.clone();
            let exists = &exists;
            async move { (!exists(pane).await).then_some(()) }
        })
        .await;
    }
    assert_eq!(windows_line(&w.ctx, "ends").await, None);

    // With remain_on_exit the pane stays, dead, even for a program that
    // ends at once, and restarts without kill.
    let kept = [
        session("kept", ends, true).await.unwrap().pane_id,
        window(ends, true).await.unwrap().pane_id,
        split(ends, true).await.unwrap().pane_id,
    ];
    for pane in &kept {
        eventually("the program to end", || async {
            (field(pane, "#{pane_dead}") == "1").then_some(())
        })
        .await;
    }
    let snapshot = functions::sessions_list(&w.ctx, EmptyInput {})
        .await
        .unwrap();
    let dead: Vec<bool> = kept
        .iter()
        .map(|id| {
            snapshot
                .sessions
                .iter()
                .flat_map(|s| &s.windows)
                .flat_map(|w| &w.panes)
                .find(|p| &p.id == id)
                .unwrap()
                .dead
        })
        .collect();
    assert_eq!(dead, vec![true, true, true], "the listing says dead too");
    // The command it was started with is the caller's, not the placeholder.
    for pane in &kept {
        assert!(
            field(pane, "#{pane_start_command}").contains("exit 3"),
            "{}",
            field(pane, "#{pane_start_command}")
        );
    }
    let respawn = |pane: &str, command: Option<&str>| {
        panes::respawn(
            &w.ctx,
            panes::RespawnPaneInput {
                pane: pane.into(),
                command: command.map(str::to_string),
                cwd: None,
                kill: None,
            },
        )
    };
    respawn(&kept[0], Some("sleep 60")).await.unwrap();
    eventually("the restarted pane to run sleep", || async {
        (field(&kept[0], "#{pane_dead}#{pane_current_command}") == "0sleep").then_some(())
    })
    .await;
    // Restarted as it was started: it ends again and stays again.
    respawn(&kept[1], None).await.unwrap();
    eventually("the restarted program to end again", || async {
        (field(&kept[1], "#{pane_dead}") == "1").then_some(())
    })
    .await;
    // Only that pane is kept (tmux 3.0 and later have pane options).
    let version = functions::status(&w.ctx, EmptyInput {})
        .await
        .unwrap()
        .version
        .unwrap();
    if tmux::tmux_cli::version_at_least(&version, 3, 0) != Some(false) {
        assert_eq!(
            raw(&[
                "show-options",
                "-p",
                "-v",
                "-t",
                &anchor.pane_id,
                "remain-on-exit"
            ]),
            ""
        );
    }
}

fn which(program: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

impl World {
    fn collector_config(&self) -> tmux::config::SharedConfig {
        WorkerConfig {
            socket_name: Some(self._server.socket.clone()),
            ..WorkerConfig::default()
        }
        .into_shared()
    }
}
