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
    let manager = TerminalManager::new(sink.clone(), collector.clone(), console, Arc::new(|| {}));

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
