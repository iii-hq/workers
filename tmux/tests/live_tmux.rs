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
    let collector = Arc::new(Collector::new(config, ConsoleClients::default()));
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
    // pane_current_path settles once the shell has started.
    eventually("both panes to report the session directory", || async {
        let snapshot = functions::sessions_list(&w.ctx, EmptyInput {}).await.ok()?;
        snapshot.sessions[0]
            .windows
            .iter()
            .all(|window| window.panes[0].cwd == "/")
            .then_some(())
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
    assert!(
        missing.contains("not found"),
        "names never prefix-match: {missing}"
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

/// A fresh folder under the system's temp folder, removed on drop.
struct Folder(std::path::PathBuf);

impl Folder {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("iii-tmux-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
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

#[tokio::test]
async fn windows_move_between_and_within_sessions() {
    if !tmux_installed() {
        eprintln!("tmux not installed; skipping");
        return;
    }
    let w = world();
    let session = |name: &str, window: &str| {
        sessions::create(
            &w.ctx,
            sessions::CreateSessionInput {
                name: Some(name.into()),
                cwd: Some("/".into()),
                command: None,
                window_name: Some(window.into()),
            },
        )
    };
    let window = |session: &str, name: &str| {
        windows::create(
            &w.ctx,
            windows::CreateWindowInput {
                session: session.into(),
                name: Some(name.into()),
                cwd: None,
                command: None,
                select: Some(false),
            },
        )
    };
    let arg = |window: &str| WindowArg::Text(window.into());
    let move_to =
        |window: &str, from: &str, to: &str, before: Option<&str>, after: Option<&str>| {
            windows::move_window(
                &w.ctx,
                windows::MoveWindowInput {
                    session: from.into(),
                    window: arg(window),
                    target_session: to.into(),
                    before: before.map(arg),
                    after: after.map(arg),
                },
            )
        };

    let src = session("src", "a0").await.unwrap();
    let a1 = window("src", "a1").await.unwrap();
    let a2 = window("src", "a2").await.unwrap();
    let dst = session("dst", "b0").await.unwrap();
    let b1 = window("dst", "b1").await.unwrap();
    window("dst", "b2").await.unwrap();
    windows::kill(
        &w.ctx,
        windows::WindowInput {
            session: "dst".into(),
            window: arg(&b1.window_id),
        },
    )
    .await
    .unwrap();
    assert_eq!(windows_line(&w.ctx, "dst").await.unwrap(), "0:b0* 2:b2");

    // A plain move goes after the last window, not into the gap at 1, and
    // leaves the destination's current window as it was.
    let moved = move_to(&a1.window_id, "src", &dst.session_id, None, None)
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

    // `after` takes the index right after its neighbour (free here).
    let moved = move_to("2", "src", "dst", None, Some("0")).await.unwrap();
    assert_eq!(
        (moved.window_id.as_str(), moved.window_index),
        (a2.window_id.as_str(), 1)
    );
    assert_eq!(
        windows_line(&w.ctx, "dst").await.unwrap(),
        "0:b0* 1:a2 2:b2 3:a1"
    );

    // Refusals change nothing.
    let refusals = [
        (
            move_to("2", "dst", "dst", None, None).await,
            "The window is already in that session.",
        ),
        (
            move_to("0", "src", "nope", None, None).await,
            "session \"nope\" not found",
        ),
        (
            move_to("0", "src", "dst", Some("0"), Some("1")).await,
            "Give before or after, not both.",
        ),
        (
            move_to("1", "dst", "dst", None, Some("1")).await,
            "A window can't be moved before or after itself.",
        ),
        (
            move_to("0", "src", "dst", None, Some("@999")).await,
            &format!("window @999 not found in session {}", dst.session_id),
        ),
        (
            move_to("7", "src", "dst", None, None).await,
            &format!("window 7 not found in session {}", src.session_id),
        ),
    ];
    for (result, expected) in refusals {
        assert_eq!(result.unwrap_err(), expected);
    }
    assert_eq!(
        windows_line(&w.ctx, "dst").await.unwrap(),
        "0:b0* 1:a2 2:b2 3:a1"
    );
    assert_eq!(windows_line(&w.ctx, "src").await.unwrap(), "0:a0*");

    // The last window of a session: tmux closes the session, which the
    // answer says; the window and its pane carry on in the other one.
    let moved = move_to(&src.window_id, &src.session_id, "dst", None, None)
        .await
        .unwrap();
    assert_eq!(
        (
            moved.window_id.as_str(),
            moved.window_index,
            moved.source_session_closed
        ),
        (src.window_id.as_str(), 4, true)
    );
    assert_eq!(windows_line(&w.ctx, "src").await, None);
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

    // Reordering the current window inside its session keeps it current.
    let moved = move_to("0", "dst", "dst", None, Some(&src.window_id))
        .await
        .unwrap();
    assert_eq!(
        (moved.window_index, moved.source_session_closed),
        (5, false)
    );
    assert_eq!(
        windows_line(&w.ctx, "dst").await.unwrap(),
        "1:a2 2:b2 3:a1 4:a0 5:b0*"
    );

    // `before` shifts the windows from its neighbour on up one index, as
    // tmux's own -b does; tmux before 3.2 has no -b and gets one sentence.
    let version = functions::status(&w.ctx, EmptyInput {})
        .await
        .unwrap()
        .version
        .unwrap();
    let before = move_to(&src.window_id, "dst", "dst", Some(&a2.window_id), None).await;
    if tmux::tmux_cli::version_at_least(&version, 3, 2) == Some(false) {
        assert_eq!(
            before.unwrap_err(),
            format!(
                "Moving a window before another needs tmux 3.2 or later, and this computer has tmux {version}."
            )
        );
        return;
    }
    let moved = before.unwrap();
    assert_eq!(moved.window_index, 1);
    assert_eq!(
        windows_line(&w.ctx, "dst").await.unwrap(),
        "1:a0 2:a2 3:b2 4:a1 6:b0*"
    );
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
    let collector = Arc::new(Collector::new(w.collector_config(), console.clone()));
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
    eventually("the echoed output", || async {
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
        String::from_utf8_lossy(&text)
            .contains("hi-from-42")
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
        },
    )
    .await
    .unwrap();
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
    assert!(missing.contains("can't find pane"), "{missing}");
    let invalid = select("@1", Some(true)).await.unwrap_err();
    assert!(invalid.contains("pane must be an id"), "{invalid}");
}

#[tokio::test]
async fn pasted_images_are_saved_for_the_terminal_that_asks() {
    use std::os::unix::fs::PermissionsExt;
    use tmux::terminal::protocol::UploadImageRequest;

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
    let folder = Folder::new();
    let paste = folder.0.join("iii/tmux/paste");
    let console = ConsoleClients::default();
    let collector = Arc::new(Collector::new(w.collector_config(), console.clone()));
    let manager = TerminalManager::new(
        Arc::new(Captured::default()),
        collector,
        console,
        Arc::new(|| {}),
        Arc::new(|| Some(false)),
    )
    .with_paste_folder(paste.clone());
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
    let reference = TerminalRef {
        terminal_id: opened.terminal_id.clone(),
        access_key: opened.access_key.clone(),
        caller_worker_id: Some(CALLER.into()),
    };
    let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01";
    let upload = |terminal: TerminalRef, mime: &str, data: &[u8]| {
        manager.upload_image(UploadImageRequest {
            terminal,
            mime: mime.into(),
            data: BASE64.encode(data),
        })
    };

    let saved = upload(reference.clone(), "image/png", png).await.unwrap();
    assert_eq!((saved.bytes, saved.mime.as_str()), (png.len(), "image/png"));
    let path = std::path::Path::new(&saved.path);
    assert_eq!(path.parent(), Some(paste.as_path()));
    assert!(path.is_absolute() && !saved.path.contains(' '));
    assert_eq!(std::fs::read(path).unwrap(), png);
    let mode = |path: &std::path::Path| path.metadata().unwrap().permissions().mode() & 0o777;
    assert_eq!((mode(path), mode(&paste)), (0o600, 0o700));
    let files = || std::fs::read_dir(&paste).unwrap().count();
    assert_eq!(files(), 1);

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
        let refused = upload(terminal, "image/png", png).await.unwrap_err();
        assert_eq!(refused, "terminal credentials are invalid");
    }
    // What isn't a PNG, JPEG, GIF or WebP image, as its first bytes say.
    assert_eq!(
        upload(reference.clone(), "image/jpeg", png)
            .await
            .unwrap_err(),
        "The image's data is not image/jpeg, as its type says."
    );
    assert_eq!(
        upload(reference.clone(), "text/plain", b"hello")
            .await
            .unwrap_err(),
        "Only PNG, JPEG, GIF and WebP images can be pasted."
    );
    assert_eq!(files(), 1, "a refusal writes nothing");

    manager.close(reference.clone()).await.unwrap();
    assert!(upload(reference, "image/png", png)
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
    assert!(missing.contains("can't find pane"), "{missing}");
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
    // A dead pane (remain-on-exit) restarts without kill, here with another command.
    raw(&[
        "set-option",
        "-w",
        "-t",
        &below.pane_id,
        "remain-on-exit",
        "on",
    ]);
    respawn(&below.pane_id, Some("sh -c 'exit 3'"), Some(true))
        .await
        .unwrap();
    eventually("the pane's program to exit", || async {
        (raw(&[
            "display-message",
            "-p",
            "-t",
            &below.pane_id,
            "#{pane_dead}",
        ]) == "1")
            .then_some(())
    })
    .await;
    respawn(&below.pane_id, Some("sleep 60"), None)
        .await
        .unwrap();
    assert_eq!(
        raw(&[
            "display-message",
            "-p",
            "-t",
            &below.pane_id,
            "#{pane_dead}"
        ]),
        "0"
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
    assert!(kill(&other.pane_id)
        .await
        .unwrap_err()
        .contains("can't find pane"));
    let killed = kill(&current.pane_id).await.unwrap();
    assert_eq!((killed.window_closed, killed.session_closed), (true, true));
    let after = functions::sessions_list(&w.ctx, EmptyInput {})
        .await
        .unwrap();
    assert!(after.sessions.is_empty());
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
