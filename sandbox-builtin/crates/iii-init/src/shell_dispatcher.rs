// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Guest-side dispatcher for the `iii.exec` virtio-console shell channel.
//!
//! Runs on a dedicated thread alongside the worker. Reads multiplexed frames from the port (one stream, many
//! concurrent sessions distinguished by `corr_id`), spawns child
//! processes in pipe mode or TTY mode, and streams their output back
//! on the same port. TTY mode allocates a pseudo-terminal via
//! `openpty(3)` and points the child's stdin/stdout/stderr at the
//! slave so `isatty(3)` says yes — needed for interactive shells,
//! line editors, and any program that toggles buffering based on
//! terminal detection.
//!
//! Threading model:
//!
//! - One reader thread (the `run` call) owns the port read half and
//!   dispatches incoming frames. It never waits on a session, so the
//!   `Credit` and `Signal` frames that unblock one are always read.
//! - Per-session stdin writer thread feeds the child (see
//!   [`spawn_stdin_writer`]); an upload has its own write thread.
//! - Per-session stdout/stderr reader threads stream child output
//!   back through the shared writer.
//! - Per-session wait thread observes child exit and sends `Exited`
//!   with the terminal flag.
//! - All frames funnel through one writer thread so they don't
//!   interleave on the wire.
//! - Flow control both ways ([`sp::SEND_WINDOW`]): a session's output
//!   waits for the host's `Credit` on its own [`Window`], and the host
//!   sends a session's input at most a window ahead of the `Credit` its
//!   stdin or write thread returns. A slow client or a child that does
//!   not read stdin stalls only its own session, and SIGKILL from the
//!   host stops it.
//!
//! Signals, stdin EOF, and cleanup are all routed by `corr_id` via the
//! shared `SessionRegistry`; session entries are removed after the
//! wait thread has emitted the terminal `Exited` frame.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread;

use iii_supervisor::shell_protocol::{
    self as sp, ShellMessage,
    flags::{FLAG_CONTROL, FLAG_TERMINAL},
};

/// One live exec session tracked by the dispatcher.
///
/// The only state we keep per session is what inbound-frame handling
/// needs: the pid (to deliver signals) and the session's stdin queue.
/// Stdout/stderr reader threads, the stdin writer and the waiter thread
/// own their own `Child` pieces and finish independently — the waiter
/// removes the entry from the registry after emitting the terminal frame.
struct SessionHandle {
    pid: u32,
    /// Input queue of the session's stdin writer thread; see
    /// [`spawn_stdin_writer`].
    stdin: SyncSender<Vec<u8>>,
    /// TTY-mode master fd, for `Resize`. `None` in pipe mode.
    master: Option<Arc<Mutex<File>>>,
    /// Set by the waiter after `exit_rx.recv()` returns. Once true
    /// the pid has been reaped and the kernel may recycle the pgid,
    /// so `kill(-pid, sig)` could land on an unrelated process group.
    terminated: bool,
}

type SessionRegistry = Arc<Mutex<HashMap<u32, SessionHandle>>>;

/// Registry for in-flight `WriteStart` streaming sessions.
///
/// Key: `corr_id`. Value: a sender that the dispatcher uses to forward
/// `FsChunk` / `FsEnd` frames to the write handler thread. The write
/// thread removes its own entry from the registry when it finishes
/// (success or error) so subsequent frames for that id fall through to
/// the catch-all `_ => {}` arm silently.
type FsWriteRegistry = Arc<Mutex<HashMap<u32, std::sync::mpsc::SyncSender<ShellMessage>>>>;

/// A session's output credit: how many more output frames it may send before
/// the host takes some of them (see [`sp::SEND_WINDOW`]). Its producers wait
/// here, so a client that reads slowly holds back only its own session.
pub(crate) struct Window {
    /// Frames it may still send, and whether the host cancelled the session.
    state: Mutex<(u32, bool)>,
    granted: Condvar,
}

impl Window {
    pub(crate) fn new(credit: u32) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new((credit, false)),
            granted: Condvar::new(),
        })
    }

    /// Takes one frame of credit, waiting for the host to grant it. `false`
    /// once the host cancelled the session: the producer stops sending.
    pub(crate) fn acquire(&self) -> bool {
        let mut state = self.state.lock().expect("window mutex poisoned");
        while state.0 == 0 && !state.1 {
            state = self.granted.wait(state).expect("window mutex poisoned");
        }
        if state.1 {
            return false;
        }
        state.0 -= 1;
        true
    }

    pub(crate) fn grant(&self, frames: u32) {
        let mut state = self.state.lock().expect("window mutex poisoned");
        state.0 = state.0.saturating_add(frames);
        self.granted.notify_all();
    }

    pub(crate) fn cancel(&self) {
        self.state.lock().expect("window mutex poisoned").1 = true;
        self.granted.notify_all();
    }
}

/// Live sessions' windows by `corr_id`, for `Credit` and `Signal` frames.
/// Weak: an entry dies with the last producer of its session.
type Windows = Arc<Mutex<HashMap<u32, Weak<Window>>>>;

fn open_window(windows: &Windows, corr_id: u32) -> Arc<Window> {
    let window = Window::new(sp::SEND_WINDOW);
    let mut map = windows.lock().expect("windows mutex poisoned");
    map.retain(|_, w| w.strong_count() > 0);
    map.insert(corr_id, Arc::downgrade(&window));
    window
}

fn window_of(windows: &Windows, corr_id: u32) -> Option<Arc<Window>> {
    let map = windows.lock().expect("windows mutex poisoned");
    map.get(&corr_id).and_then(Weak::upgrade)
}

/// Writer handle shared by every thread that emits frames. A single
/// dedicated writer thread drains this channel onto the virtio-console
/// port, so no caller ever holds a lock across a virtio write. Each
/// session's output waits for its own [`Window`] first, so the channel
/// holds at most a window of any one session's frames; when it is full
/// anyway, senders wait for room (see [`send_frame`]); once the writer
/// thread is gone, sends fail.
pub(crate) type Writer = SyncSender<Vec<u8>>;

/// Bound on queued frames between session threads and the writer
/// thread. Roughly 1024 × 8 KiB worst case ≈ 8 MiB in flight. Large
/// enough that a short virtio stall never makes a sender wait; small enough
/// that a permanent stall (e.g. host relay dead) can't balloon guest
/// memory. Chosen empirically, not tuned.
const WRITER_CHANNEL_CAPACITY: usize = 1024;

/// How many bytes of child output we ship per frame. Chosen to balance
/// overhead (frame header + JSON envelope + base64 expansion) against
/// latency — 8 KiB lets `echo` + `cat` feel snappy without allocating
/// megabyte buffers for a wedged `tail -f`.
const IO_CHUNK_SIZE: usize = 8 * 1024;

/// Open `port_path` and run the dispatcher loop until the host closes
/// the channel. Blocks the calling thread — the caller is expected to
/// be a dedicated thread spawned from `iii-init`.
///
/// Returns `Ok(())` on clean EOF, `Err` if the port itself fails
/// (rare — a wedged session produces a per-session `Error` frame and
/// does not bring the dispatcher down).
pub fn run(port_path: &Path) -> anyhow::Result<()> {
    let writer_file = OpenOptions::new().read(true).write(true).open(port_path)?;
    let reader_file = writer_file.try_clone()?;
    let writer = spawn_writer_thread(writer_file);
    let sessions: SessionRegistry = Arc::new(Mutex::new(HashMap::new()));
    // Registry for in-flight WriteStart sessions. Keyed by corr_id;
    // value is a sender that forwards FsChunk/FsEnd to the write thread.
    let fs_writes: FsWriteRegistry = Arc::new(Mutex::new(HashMap::new()));
    let windows = Windows::default();

    let mut reader = BufReader::new(reader_file);
    loop {
        match sp::read_frame_blocking(&mut reader) {
            Ok(Some((corr_id, _flags, msg))) => {
                handle_frame(corr_id, msg, &sessions, &writer, &fs_writes, &windows);
            }
            Ok(None) => break,
            Err(e) => {
                // A frame-level error (bad length, truncated body) is
                // usually fatal for this channel — the peer is out of
                // sync. Log and drop the reader; sessions already
                // running keep writing to the port, which the host
                // relay will also bail on once it sees the same
                // framing trouble.
                eprintln!("iii-init: shell dispatcher read error: {e}");
                break;
            }
        }
    }
    Ok(())
}

/// Spawn the single writer thread that owns the virtio-console port's
/// write half. All session threads push frames into its channel; the
/// thread drains them serially, so no other thread ever blocks on
/// virtio I/O. Mirrors the host relay's `vm_writer_task` design.
///
/// The returned [`SyncSender`] is the sole handle callers use to emit
/// frames; dropping all clones closes the channel and the writer
/// thread exits cleanly. If `write_all` fails (port closed, host
/// relay torn down), the thread logs and exits — further session
/// sends silently no-op as the channel becomes disconnected.
fn spawn_writer_thread(mut file: File) -> Writer {
    let (tx, rx) = sync_channel::<Vec<u8>>(WRITER_CHANNEL_CAPACITY);
    thread::Builder::new()
        .name("iii-init-shell-writer".to_string())
        .spawn(move || {
            while let Ok(frame) = rx.recv() {
                if let Err(e) = file.write_all(&frame) {
                    eprintln!("iii-init: shell dispatcher writer failed: {e}");
                    break;
                }
            }
        })
        .expect("spawn writer thread");
    tx
}

fn handle_frame(
    corr_id: u32,
    msg: ShellMessage,
    sessions: &SessionRegistry,
    writer: &Writer,
    fs_writes: &FsWriteRegistry,
    windows: &Windows,
) {
    match msg {
        ShellMessage::Request {
            cmd,
            args,
            env,
            cwd,
            tty,
            rows,
            cols,
        } => {
            let window = open_window(windows, corr_id);
            let spawn_result = if tty {
                spawn_tty_session(
                    corr_id, cmd, args, env, cwd, rows, cols, sessions, writer, window,
                )
            } else {
                spawn_pipe_session(corr_id, cmd, args, env, cwd, sessions, writer, window)
            };
            if let Err(e) = spawn_result {
                send_frame(
                    writer,
                    corr_id,
                    FLAG_TERMINAL,
                    &ShellMessage::Error {
                        message: format!("spawn: {e}"),
                    },
                );
            }
        }
        ShellMessage::Stdin { data_b64 } => {
            let decoded = match decode_b64(&data_b64) {
                Ok(d) => d,
                Err(_) => return,
            };
            let session = {
                let map = sessions.lock().expect("session map mutex poisoned");
                map.get(&corr_id)
                    .filter(|s| !s.terminated)
                    .map(|s| (s.stdin.clone(), s.pid))
            };
            // Never waits: the queue holds the window the host may send.
            // A host past it gets its session killed, not a dropped frame.
            // Disconnected: the child's stdin is already closed (EOF).
            if let Some((stdin, pid)) = session
                && let Err(TrySendError::Full(_)) = stdin.try_send(decoded)
            {
                eprintln!("iii-init: corr_id={corr_id} sent stdin past its window; killing it");
                unsafe {
                    libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
                }
            }
        }
        ShellMessage::Signal { signal } => {
            // Refuse delivery if terminated: PID 1 has reaped and the
            // pgid may be recycled, so `kill(-pid, sig)` could hit an
            // unrelated process group.
            let pid = {
                let map = sessions.lock().expect("session map mutex poisoned");
                map.get(&corr_id).filter(|s| !s.terminated).map(|s| s.pid)
            };
            if let Some(pid) = pid
                && signal > 0
            {
                // kill(2) is async-signal-safe and never blocks. We
                // don't care about the result — ESRCH means the
                // child already died, and the waiter thread will
                // have emitted Exited or is about to.
                //
                // Signal the whole process group (-pid) so pipelines
                // and other multi-process children inside `sh -c`
                // receive the signal too. Both spawn paths make the
                // child a pgroup leader (pipe mode via setpgid(0,0)
                // in pre_exec, TTY mode via setsid()), so pgid == pid.
                unsafe {
                    libc::kill(-(pid as libc::pid_t), signal as libc::c_int);
                }
            }

            // SIGKILL on a corr_id is also the relay's "host
            // disconnected, abandon any in-flight session" signal
            // (see shell_relay.rs::client_session cleanup, which
            // fans out Signal{9} for every owned corr_id when the
            // host disconnects). For exec sessions the libc::kill
            // above terminates the child; for in-flight FS write
            // sessions we drop the chunk_tx so handle_write_start's
            // recv() returns Disconnected, unlinks the temp file,
            // and returns S218 promptly. Without this, the temp
            // leaks for up to WRITE_IDLE_TIMEOUT (30s) before
            // streaming.rs's safety valve fires — too late for any
            // caller-side test that asserts cleanup at trigger
            // timeout.
            // A read or an exec's output streams stop too, so the guest
            // sends no more frames for the abandoned session.
            if signal == libc::SIGKILL {
                let _ = fs_writes
                    .lock()
                    .expect("fs_writes mutex poisoned")
                    .remove(&corr_id);
                if let Some(window) = window_of(windows, corr_id) {
                    window.cancel();
                }
            }
        }
        ShellMessage::Credit { frames } => {
            if let Some(window) = window_of(windows, corr_id) {
                window.grant(frames);
            }
        }
        ShellMessage::Resize { rows, cols } => {
            let master = {
                let map = sessions.lock().expect("session map mutex poisoned");
                map.get(&corr_id)
                    .filter(|s| !s.terminated)
                    .and_then(|s| s.master.clone())
            };
            if let Some(master) = master {
                let ws = libc::winsize {
                    ws_row: rows,
                    ws_col: cols,
                    ws_xpixel: 0,
                    ws_ypixel: 0,
                };
                let guard = master.lock().expect("master mutex poisoned");
                // Best-effort: if TIOCSWINSZ fails (unlikely on a
                // healthy PTY master) the child keeps its previous
                // size; nothing else we can do without tearing down
                // the session.
                unsafe {
                    libc::ioctl(guard.as_raw_fd(), libc::TIOCSWINSZ, &ws);
                }
            }
        }
        // ── Filesystem ops ────────────────────────────────────────────────
        ShellMessage::FsRequest(op) => {
            use iii_shell_proto::FsOp;
            use iii_shell_proto::flags::FLAG_TERMINAL;

            match op {
                // ── WriteStart: streaming upload ─────────────────────────
                FsOp::WriteStart {
                    path,
                    mode,
                    parents,
                } => {
                    // Create a sync channel for this write session and
                    // register the sender so FsChunk/FsEnd frames can
                    // be forwarded to the write thread. It holds the
                    // window the host may send ahead of our Credit.
                    let (chunk_tx, chunk_rx) =
                        sync_channel::<ShellMessage>(sp::SEND_WINDOW as usize);
                    fs_writes
                        .lock()
                        .expect("fs_writes mutex poisoned")
                        .insert(corr_id, chunk_tx);

                    let writer = writer.clone();
                    let fs_writes = fs_writes.clone();
                    thread::Builder::new()
                        .name(format!("iii-fs-write-{corr_id}"))
                        .spawn(move || {
                            let credit = || {
                                send_frame(&writer, corr_id, FLAG_CONTROL, &CREDIT_ONE);
                            };
                            let result = crate::fs_handler::streaming::handle_write_start(
                                path, mode, parents, chunk_rx, &credit,
                            );
                            // Always remove the registry entry when done.
                            fs_writes
                                .lock()
                                .expect("fs_writes mutex poisoned")
                                .remove(&corr_id);
                            match result {
                                Ok(fs_result) => {
                                    send_frame(
                                        &writer,
                                        corr_id,
                                        FLAG_TERMINAL,
                                        &ShellMessage::FsResponse(fs_result),
                                    );
                                }
                                Err(e) => {
                                    send_frame(
                                        &writer,
                                        corr_id,
                                        FLAG_TERMINAL,
                                        &ShellMessage::FsError {
                                            code: e.code.to_string(),
                                            message: e.message,
                                        },
                                    );
                                }
                            }
                        })
                        .ok();
                }

                // ── ReadStart: streaming download ─────────────────────────
                FsOp::ReadStart { path } => {
                    let writer = writer.clone();
                    let window = open_window(windows, corr_id);
                    thread::Builder::new()
                        .name(format!("iii-fs-read-{corr_id}"))
                        .spawn(move || {
                            let result = crate::fs_handler::streaming::handle_read_start(
                                path, &writer, corr_id, &window,
                            );
                            // handle_read_start only returns Err for
                            // pre-FsMeta failures; emit the terminal
                            // FsError here.
                            if let Err(e) = result {
                                send_frame(
                                    &writer,
                                    corr_id,
                                    FLAG_TERMINAL,
                                    &ShellMessage::FsError {
                                        code: e.code.to_string(),
                                        message: e.message,
                                    },
                                );
                            }
                        })
                        .ok();
                }

                // ── One-shot ops ──────────────────────────────────────────
                one_shot => {
                    let writer = writer.clone();
                    thread::Builder::new()
                        .name(format!("iii-fs-op-{corr_id}"))
                        .spawn(move || {
                            let result = dispatch_one_shot(one_shot);
                            match result {
                                Ok(fs_result) => {
                                    send_frame(
                                        &writer,
                                        corr_id,
                                        FLAG_TERMINAL,
                                        &ShellMessage::FsResponse(fs_result),
                                    );
                                }
                                Err(e) => {
                                    send_frame(
                                        &writer,
                                        corr_id,
                                        FLAG_TERMINAL,
                                        &ShellMessage::FsError {
                                            code: e.code.to_string(),
                                            message: e.message,
                                        },
                                    );
                                }
                            }
                        })
                        .ok();
                }
            }
        }

        // ── FsChunk / FsEnd: forward to active write session ──────────────
        ShellMessage::FsChunk { .. } | ShellMessage::FsEnd => {
            let sender = fs_writes
                .lock()
                .expect("fs_writes mutex poisoned")
                .get(&corr_id)
                .cloned();
            if let Some(tx) = sender {
                // try_send never blocks the dispatcher loop, and the
                // queue holds the window the host may send ahead of the
                // write thread's Credit, so only a host past its window
                // finds it full. Dropping that chunk while a later
                // FsEnd still arrives would rename a truncated temp
                // file into place with a success reply. Abort the
                // session instead: drop the registry entry so the
                // writer thread observes Disconnected on its next
                // recv_timeout, unlinks the temp file, and emits a
                // terminal FsError(S218). Subsequent FsChunk/FsEnd
                // frames for this corr_id find no sender and are
                // silently ignored — correct, the session is over.
                if let Err(e) = tx.try_send(msg) {
                    match e {
                        TrySendError::Full(_) => {
                            eprintln!(
                                "iii-init: corr_id={corr_id} sent an upload past its \
                                 window; aborting it"
                            );
                            fs_writes
                                .lock()
                                .expect("fs_writes mutex poisoned")
                                .remove(&corr_id);
                        }
                        TrySendError::Disconnected(_) => {
                            // Write thread already exited — ignore.
                        }
                    }
                }
            }
            // If no entry exists the write session already finished or
            // was never started — silently drop.
        }

        // Host-only variants received from the host are ignored; only
        // Request/Stdin/Signal/Resize/FsRequest/FsChunk/FsEnd make sense
        // as inbound. Anything else is a peer protocol error — dropping
        // is less noisy than responding with Error and keeps the channel
        // alive.
        _ => {}
    }
}

/// Dispatch a one-shot `FsOp` variant to the corresponding handler.
/// `WriteStart` and `ReadStart` are handled by the streaming path and
/// must never reach this function.
fn dispatch_one_shot(op: iii_shell_proto::FsOp) -> crate::fs_handler::FsCallResult {
    use iii_shell_proto::FsOp;
    match op {
        FsOp::Ls { path } => crate::fs_handler::ops::ls(path),
        FsOp::Stat { path } => crate::fs_handler::ops::stat(path),
        FsOp::Mkdir {
            path,
            mode,
            parents,
        } => crate::fs_handler::ops::mkdir(path, mode, parents),
        FsOp::Rm { path, recursive } => crate::fs_handler::ops::rm(path, recursive),
        FsOp::Chmod {
            path,
            mode,
            uid,
            gid,
            recursive,
        } => crate::fs_handler::ops::chmod(path, mode, uid, gid, recursive),
        FsOp::Mv {
            src,
            dst,
            overwrite,
        } => crate::fs_handler::ops::mv(src, dst, overwrite),
        FsOp::Grep {
            path,
            pattern,
            recursive,
            ignore_case,
            include_glob,
            exclude_glob,
            max_matches,
            max_line_bytes,
        } => crate::fs_handler::ops::grep(
            path,
            pattern,
            recursive,
            ignore_case,
            include_glob,
            exclude_glob,
            max_matches,
            max_line_bytes,
        ),
        FsOp::Sed {
            files,
            path,
            recursive,
            include_glob,
            exclude_glob,
            pattern,
            replacement,
            regex,
            first_only,
            ignore_case,
        } => crate::fs_handler::ops::sed(
            files,
            path,
            recursive,
            include_glob,
            exclude_glob,
            pattern,
            replacement,
            regex,
            first_only,
            ignore_case,
        ),
        // Streaming ops are dispatched before this function is called.
        FsOp::WriteStart { .. } | FsOp::ReadStart { .. } => Err(crate::fs_handler::FsError::new(
            "S219",
            "streaming ops must not reach dispatch_one_shot",
        )),
    }
}

#[allow(clippy::too_many_arguments, clippy::zombie_processes)]
fn spawn_pipe_session(
    corr_id: u32,
    cmd: String,
    args: Vec<String>,
    env: Vec<String>,
    cwd: Option<String>,
    sessions: &SessionRegistry,
    writer: &Writer,
    window: Arc<Window>,
) -> anyhow::Result<()> {
    let mut command = Command::new(&cmd);
    command
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd.as_deref() {
        command.current_dir(dir);
    }
    for kv in &env {
        if let Some((k, v)) = kv.split_once('=') {
            command.env(k, v);
        }
    }
    // Put the child in its own process group so `Signal` frames can
    // reach grandchildren. Without this, `/bin/sh -c "a | b"` leaves
    // `a` and `b` in the dispatcher's pgroup, and `kill(pid, SIGINT)`
    // hits only the shell, while Ctrl-C from an exec client would
    // exit the session with pipeline stages still alive in the VM.
    // TTY mode already gets the same effect via `setsid()`.
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    // Race-free: the `child_exits` registry lock is held across
    // `spawn`, so PID 1's reap loop cannot observe and discard the
    // child's exit before we register as its owner. The inner
    // closure owns the spawn error only long enough to thread it
    // back out through the `Option` return.
    let mut spawn_err: Option<std::io::Error> = None;
    let mut maybe_child: Option<std::process::Child> = None;
    let reg = crate::child_exits::register_spawn(|| match command.spawn() {
        Ok(c) => {
            let pid = c.id();
            maybe_child = Some(c);
            Some(pid)
        }
        Err(e) => {
            spawn_err = Some(e);
            None
        }
    });
    let (pid, exit_rx) = match reg {
        Some(v) => v,
        None => {
            let e = spawn_err
                .unwrap_or_else(|| std::io::Error::other("spawn returned None without error"));
            return Err(e.into());
        }
    };
    let mut child = maybe_child.expect("register_spawn returned pid without child");
    let stdout = child.stdout.take().expect("stdout requested");
    let stderr = child.stderr.take().expect("stderr requested");
    let child_stdin = child.stdin.take().expect("stdin requested");
    let live = Arc::new(Mutex::new(true));
    let stdin = match spawn_stdin_writer(corr_id, child_stdin, writer.clone(), live.clone(), true) {
        Ok(stdin) => stdin,
        Err(e) => {
            // Its stdin is already closed: the child must not run on a
            // short input. See spawn_tty_session for the order.
            unsafe {
                libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
            }
            crate::child_exits::unregister(pid);
            return Err(e.into());
        }
    };

    // Register before announcing so a racing Stdin frame finds the
    // entry.
    sessions.lock().expect("session map mutex poisoned").insert(
        corr_id,
        SessionHandle {
            pid,
            stdin,
            master: None,
            terminated: false,
        },
    );

    send_frame(writer, corr_id, 0, &ShellMessage::Started { pid });

    // Stream stdout / stderr in dedicated threads so neither can
    // starve the other. Each thread emits Stdout/Stderr frames until
    // its pipe closes (child exit or explicit fd close). We keep the
    // `JoinHandle`s so the waiter thread can block on them before
    // emitting the terminal `Exited` frame — otherwise a fast-exiting
    // child can get its Exited frame out before the reader threads
    // have drained remaining output, and the host relay will drop
    // those late Stdout/Stderr frames (FLAG_TERMINAL on Exited
    // removes the route).
    let stdout_handle = {
        let (writer, window) = (writer.clone(), window.clone());
        thread::Builder::new()
            .name(format!("iii-exec-{corr_id}-out"))
            .spawn(move || stream_fd(writer, corr_id, stdout, OutputKind::Stdout, window))
            .ok()
    };
    let stderr_handle = {
        let writer = writer.clone();
        thread::Builder::new()
            .name(format!("iii-exec-{corr_id}-err"))
            .spawn(move || stream_fd(writer, corr_id, stderr, OutputKind::Stderr, window))
            .ok()
    };

    // Waiter thread: the PID-1 reap loop (in `supervisor.rs`) will
    // observe the child's exit and forward the code via `exit_rx`.
    // We deliberately do NOT call `child.wait()` — PID 1 is the only
    // reaper; calling wait here would race with it and hang. Instead
    // we receive the exit code from the shared `child_exits`
    // registry and drop the `Child` handle after: by then the kernel
    // has already reaped the process, and `Drop for Child` is a
    // no-op on an already-dead child.
    //
    // Before emitting the terminal `Exited` frame we join the
    // stdout/stderr reader threads so the host sees every byte of
    // child output before the route is torn down.
    {
        let writer = writer.clone();
        let sessions = sessions.clone();
        thread::Builder::new()
            .name(format!("iii-exec-{corr_id}-wait"))
            .spawn(move || {
                let code = exit_rx.recv().unwrap_or(-1);
                // Mark terminated BEFORE joining readers — reader
                // joins can block while guest drains output, and the
                // kernel can recycle the reaped pgid in that window.
                mark_terminated(&sessions, corr_id, pid);
                drop(child);
                if let Some(h) = stdout_handle {
                    let _ = h.join();
                }
                if let Some(h) = stderr_handle {
                    let _ = h.join();
                }
                *live.lock().expect("live mutex poisoned") = false;
                send_frame(
                    &writer,
                    corr_id,
                    FLAG_TERMINAL,
                    &ShellMessage::Exited { code },
                );
                // pid-match before removing: if the host side has
                // already reassigned this corr_id to a new session
                // (e.g. after a client disconnect + reclaim), the
                // entry here belongs to someone else. Clobbering it
                // would orphan the new session in the registry and
                // cause Signal/Resize/Stdin frames for it to drop.
                remove_session_if_owned(&sessions, corr_id, pid);
            })
            .ok();
    }

    Ok(())
}

/// Spawn a child attached to a freshly-allocated PTY. The slave side
/// becomes the child's stdin/stdout/stderr and its controlling
/// terminal; the master side stays on the dispatcher for stream-out
/// and Resize/Stdin plumbing.
///
/// Unlike the pipe path, TTY sessions have a single reader thread
/// (the PTY merges stdout and stderr onto the master) and emit all
/// output as `ShellMessage::Stdout`. If the user cares about separate
/// streams they should use pipe mode.
///
/// clippy::zombie_processes is disabled because PID 1 is the reaper,
/// not us. The waiter thread receives the exit code via the shared
/// `child_exits` registry and drops the `Child` handle after — by
/// then the kernel has reaped. See the comment on the waiter thread
/// in `spawn_pipe_session` for the full rationale.
#[allow(clippy::too_many_arguments, clippy::zombie_processes)]
fn spawn_tty_session(
    corr_id: u32,
    cmd: String,
    args: Vec<String>,
    env: Vec<String>,
    cwd: Option<String>,
    rows: u16,
    cols: u16,
    sessions: &SessionRegistry,
    writer: &Writer,
    window: Arc<Window>,
) -> anyhow::Result<()> {
    use nix::pty::openpty;

    let pty = openpty(None, None)?;
    let master_owned = pty.master;
    let slave_owned = pty.slave;

    // Apply the initial window size on the master before spawning —
    // this becomes the child's TTY size via the slave it inherits.
    // TIOCSWINSZ on the master is the canonical way to resize a PTY
    // session and triggers SIGWINCH delivery to the foreground group.
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let ret = unsafe { libc::ioctl(master_owned.as_raw_fd(), libc::TIOCSWINSZ, &ws) };
    if ret < 0 {
        return Err(std::io::Error::last_os_error().into());
    }

    // The child inherits the slave three times via Stdio::from, which
    // takes ownership. Dup so we hand out three independent fds and
    // keep one last copy for the pre_exec controlling-tty dance.
    let slave_stdin = slave_owned.try_clone()?;
    let slave_stdout = slave_owned.try_clone()?;
    let slave_stderr = slave_owned;

    let mut command = Command::new(&cmd);
    command
        .args(&args)
        .stdin(Stdio::from(slave_stdin))
        .stdout(Stdio::from(slave_stdout))
        .stderr(Stdio::from(slave_stderr));
    if let Some(dir) = cwd.as_deref() {
        command.current_dir(dir);
    }
    for kv in &env {
        if let Some((k, v)) = kv.split_once('=') {
            command.env(k, v);
        }
    }
    // pre_exec runs in the child between fork and the spawn's final
    // syscall. Only async-signal-safe syscalls allowed — no
    // allocations, no Rust collections, no locks.
    //
    // Step 1: setsid() — detach from the parent's session so the
    //   child becomes a session leader and can acquire its own
    //   controlling terminal.
    // Step 2: TIOCSCTTY on fd 0 — which is already the slave thanks
    //   to Stdio::from above — makes that slave the session's
    //   controlling TTY. From here `isatty(0)` says yes and job
    //   control works (SIGINT on Ctrl-C, etc).
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    // Race-free spawn (same pattern as pipe mode — see the long note
    // on `register_spawn` there).
    let mut spawn_err: Option<std::io::Error> = None;
    let mut maybe_child: Option<std::process::Child> = None;
    let reg = crate::child_exits::register_spawn(|| match command.spawn() {
        Ok(c) => {
            let pid = c.id();
            maybe_child = Some(c);
            Some(pid)
        }
        Err(e) => {
            spawn_err = Some(e);
            None
        }
    });
    let (pid, exit_rx) = match reg {
        Some(v) => v,
        None => {
            let e = spawn_err
                .unwrap_or_else(|| std::io::Error::other("spawn returned None without error"));
            return Err(e.into());
        }
    };
    let child = maybe_child.expect("register_spawn returned pid without child");

    // Master end: wrap as File so we get blocking Read/Write. Dup it
    // for the reader thread and the stdin writer; the original goes
    // into the session map (Arc<Mutex<File>>) for Resize.
    //
    // try_clone can fail (EMFILE under FD pressure). At this point the
    // child is already spawned and registered with `child_exits`, so a
    // bare `?` would orphan: PID 1 would dispatch the exit code to an
    // `exit_rx` nobody's receiving on and the registry entry would
    // leak.
    //
    // Cleanup order matters: SIGKILL FIRST (while the registry still
    // claims this pid, so PID 1 routes the exit code into the sender
    // we're about to drop, rather than invoking its default orphan
    // reap — which in the tiny window between unregister and kill
    // could let the kernel recycle the pid to a different process and
    // deliver our SIGKILL to the wrong target). THEN unregister,
    // which drops the sender — any buffered code is discarded.
    // `Child::drop` is a no-op on Unix (no waitpid), so no extra step
    // is needed for the handle; it falls out of scope at return.
    let master_file: File = unsafe { File::from_raw_fd(master_owned.into_raw_fd()) };
    let live = Arc::new(Mutex::new(true));
    let fds = master_file.try_clone().and_then(|reader| {
        let input = master_file.try_clone()?;
        let stdin = spawn_stdin_writer(corr_id, input, writer.clone(), live.clone(), false)?;
        Ok((reader, stdin))
    });
    let (master_reader, stdin) = match fds {
        Ok(fds) => fds,
        Err(e) => {
            // Kill the whole session (setsid() made pid the pgroup
            // leader, so -pid targets every descendant spawned before
            // the TTY cleanup raced in).
            unsafe {
                libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
            }
            crate::child_exits::unregister(pid);
            let _ = child;
            return Err(e.into());
        }
    };
    let master = Arc::new(Mutex::new(master_file));

    sessions.lock().expect("session map mutex poisoned").insert(
        corr_id,
        SessionHandle {
            pid,
            stdin,
            master: Some(master.clone()),
            terminated: false,
        },
    );

    send_frame(writer, corr_id, 0, &ShellMessage::Started { pid });

    // One reader thread: PTY merges stdout and stderr onto the
    // master, so we emit everything as Stdout frames. The reader
    // exits when the master sees EIO (child exited + slave closed).
    let output_handle = {
        let writer = writer.clone();
        thread::Builder::new()
            .name(format!("iii-exec-{corr_id}-tty"))
            .spawn(move || stream_fd(writer, corr_id, master_reader, OutputKind::Stdout, window))
            .ok()
    };

    // Waiter thread: same contract as pipe mode — join the reader so
    // no output frames race past the terminal Exited frame.
    {
        let writer = writer.clone();
        let sessions = sessions.clone();
        thread::Builder::new()
            .name(format!("iii-exec-{corr_id}-wait"))
            .spawn(move || {
                let code = exit_rx.recv().unwrap_or(-1);
                mark_terminated(&sessions, corr_id, pid);
                drop(child);
                if let Some(h) = output_handle {
                    let _ = h.join();
                }
                *live.lock().expect("live mutex poisoned") = false;
                send_frame(
                    &writer,
                    corr_id,
                    FLAG_TERMINAL,
                    &ShellMessage::Exited { code },
                );
                // See spawn_pipe_session for the ownership-check
                // rationale.
                remove_session_if_owned(&sessions, corr_id, pid);
            })
            .ok();
    }

    Ok(())
}

/// Mark a session terminated iff the entry still belongs to
/// `expected_pid`. Pid check guards against a host-side reclaim that
/// assigned the same `corr_id` to a new session.
fn mark_terminated(sessions: &SessionRegistry, corr_id: u32, expected_pid: u32) {
    let mut map = sessions.lock().expect("session map mutex poisoned");
    if let Some(entry) = map.get_mut(&corr_id)
        && entry.pid == expected_pid
    {
        entry.terminated = true;
    }
}

/// Remove `corr_id` from the session registry iff the entry still
/// belongs to `expected_pid`. Protects against clobbering a freshly
/// installed session that reused the same correlation id after a
/// host-side reclaim.
fn remove_session_if_owned(sessions: &SessionRegistry, corr_id: u32, expected_pid: u32) {
    let mut map = sessions.lock().expect("session map mutex poisoned");
    if let Some(entry) = map.get(&corr_id)
        && entry.pid == expected_pid
    {
        map.remove(&corr_id);
    }
}

enum OutputKind {
    Stdout,
    Stderr,
}

fn stream_fd<R: Read>(
    writer: Writer,
    corr_id: u32,
    mut src: R,
    kind: OutputKind,
    window: Arc<Window>,
) {
    let mut buf = vec![0u8; IO_CHUNK_SIZE];
    loop {
        match src.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                // Until the host takes earlier output, the child blocks on a
                // full pipe; once the host cancelled, stop reading it.
                if !window.acquire() {
                    break;
                }
                let encoded = encode_b64(&buf[..n]);
                let msg = match kind {
                    OutputKind::Stdout => ShellMessage::Stdout { data_b64: encoded },
                    OutputKind::Stderr => ShellMessage::Stderr { data_b64: encoded },
                };
                send_frame(&writer, corr_id, 0, &msg);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

/// The guest's input credit: one more `Stdin`/`FsChunk` frame taken.
const CREDIT_ONE: ShellMessage = ShellMessage::Credit { frames: 1 };

/// Feeds a session's stdin on its own thread, so the dispatcher never waits
/// on a child that is slow to read: a blocking write there would also hold up
/// the `Credit` and `Signal` frames that unblock this session and every other.
/// Each frame taken earns the host one more ([`sp::SEND_WINDOW`]). An empty
/// frame closes the child's stdin when `eof_closes` (pipe mode); in a TTY, EOF
/// is the Ctrl-D byte, and closing would SIGHUP the child.
///
/// `live` goes false before the session's terminal frame: no credit may
/// follow it, or it could reach a later session on the same `corr_id`.
fn spawn_stdin_writer<W: Write + Send + 'static>(
    corr_id: u32,
    mut child_stdin: W,
    writer: Writer,
    live: Arc<Mutex<bool>>,
    eof_closes: bool,
) -> std::io::Result<SyncSender<Vec<u8>>> {
    let (tx, rx) = sync_channel::<Vec<u8>>(sp::SEND_WINDOW as usize);
    thread::Builder::new()
        .name(format!("iii-exec-{corr_id}-in"))
        .spawn(move || {
            // Once the child stops reading stdin, keep taking frames so the
            // host can finish sending; the child sees what it read.
            let mut closed = false;
            while let Ok(bytes) = rx.recv() {
                let eof = bytes.is_empty();
                if !eof && !closed {
                    closed = child_stdin.write_all(&bytes).is_err();
                }
                let live = live.lock().expect("live mutex poisoned");
                if *live {
                    send_frame(&writer, corr_id, FLAG_CONTROL, &CREDIT_ONE);
                }
                drop(live);
                if eof && eof_closes {
                    break;
                }
            }
        })?;
    Ok(tx)
}

/// Queue one frame for the writer thread; `false` once that thread is gone.
pub(crate) fn send_frame(writer: &Writer, corr_id: u32, flags: u8, msg: &ShellMessage) -> bool {
    let frame = match sp::encode_frame(corr_id, flags, msg) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("iii-init: shell dispatcher encode failed: {e}");
            return false;
        }
    };
    // Waits for room rather than dropping the frame: a dropped frame was
    // lost output, a short fs::read, or a lost `Exited` that left an exec
    // hanging. The host relay never stops reading the port, and each
    // session's frames are bounded by its windows, so the wait is short.
    // The writer thread waits on nothing but the port, and the dispatcher
    // thread on no session, so it cannot deadlock.
    writer.send(frame).is_ok()
}

fn encode_b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode_b64(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::sync::OnceLock;
    use std::time::Duration;

    /// Skip with a visible log line instead of a silent `return` so
    /// a distroless CI image can't report green with zero assertions.
    fn skip_if_missing(binary: &str) -> bool {
        if !std::path::Path::new(binary).exists() {
            eprintln!("[skip] shell_dispatcher test: {binary} not present");
            return true;
        }
        false
    }

    /// Serialize spawning tests: the shared OnceLock reaper calls
    /// `waitpid(-1, WNOHANG)` process-wide, so parallel tests race on
    /// `child_exits::dispatch_exit`.
    fn serial_spawn_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Process-wide reap thread for tests.
    ///
    /// Production runs PID 1's `waitpid(-1)` loop in `supervisor.rs`, and
    /// that loop calls `child_exits::dispatch_exit` to unblock session
    /// waiter threads. The unit tests here bypass `supervisor::run_*`
    /// and call `handle_frame` directly, so without this helper every
    /// child becomes a zombie, every waiter thread blocks on
    /// `exit_rx.recv()`, the terminal `Exited` frame never fires, and
    /// the host's read times out with `WouldBlock`.
    ///
    /// Started lazily via `OnceLock`: the first test to spawn children
    /// kicks off a single detached reaper thread that lives until the
    /// test process exits. Repeated calls are no-ops.
    #[cfg(target_os = "linux")]
    fn ensure_test_reaper() {
        static STARTED: OnceLock<()> = OnceLock::new();
        STARTED.get_or_init(|| {
            use nix::sys::wait::WaitStatus;
            thread::Builder::new()
                .name("shell-dispatcher-test-reaper".to_string())
                .spawn(|| {
                    loop {
                        // PID 1's own reap (registry-locked), not a bare waitpid(-1): that
                        // reaped other tests' failed spawns before std waited for them.
                        match crate::child_exits::wait_any() {
                            Ok(WaitStatus::Exited(pid, code)) => {
                                crate::child_exits::dispatch_exit(pid.as_raw(), code);
                            }
                            Ok(WaitStatus::Signaled(pid, sig, _)) => {
                                crate::child_exits::dispatch_exit(pid.as_raw(), 128 + sig as i32);
                            }
                            // Stopped/Continued/PtraceEvent/PtraceSyscall —
                            // not terminal, leave the child alone.
                            Ok(_) => {}
                            // ECHILD (no children yet), EINTR or another transient
                            // errno; brief pause and retry so we don't busy-loop.
                            Err(_) => thread::sleep(Duration::from_millis(10)),
                        }
                    }
                })
                .expect("spawn test reaper");
        });
    }

    /// Frames wait for room in a full writer channel instead of being
    /// dropped, so an exec's output and its `Exited` survive other
    /// sessions keeping the channel full.
    #[test]
    fn send_frame_waits_for_a_full_writer_instead_of_dropping() {
        let (writer, frames) = sync_channel::<Vec<u8>>(1);
        let sender = thread::spawn(move || {
            (0..50).all(|code| send_frame(&writer, 1, 0, &ShellMessage::Exited { code }))
        });
        // A slow host: the channel is long full before it drains.
        thread::sleep(Duration::from_millis(50));
        assert_eq!(frames.iter().count(), 50);
        assert!(sender.join().unwrap());
    }

    /// The host's Credit and SIGKILL frames reach a streaming read: it sends
    /// a window of chunks, one more per credited frame, and stops when killed.
    #[test]
    fn credit_and_sigkill_reach_a_read() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("big.bin");
        let window = sp::SEND_WINDOW as usize;
        std::fs::write(&file, vec![1u8; 4 * window * 64 * 1024]).unwrap();
        let (writer, frames) = sync_channel::<Vec<u8>>(1024);
        let (sessions, fs_writes) = (SessionRegistry::default(), FsWriteRegistry::default());
        let windows = Windows::default();
        let send = |msg| handle_frame(5, msg, &sessions, &writer, &fs_writes, &windows);
        let next = || {
            thread::sleep(Duration::from_millis(200));
            let got: Vec<Vec<u8>> = frames.try_iter().collect();
            got.iter()
                .map(|f| sp::decode_frame_body(&f[4..]).unwrap())
                .collect::<Vec<_>>()
        };
        let chunks = |got: &[(u32, u8, ShellMessage)]| {
            got.iter()
                .filter(|m| matches!(m.2, ShellMessage::FsChunk { .. }))
                .count()
        };

        send(ShellMessage::FsRequest(iii_shell_proto::FsOp::ReadStart {
            path: file.to_string_lossy().into_owned(),
        }));
        assert_eq!(chunks(&next()), window);
        send(ShellMessage::Credit { frames: 3 });
        assert_eq!(chunks(&next()), 3);
        send(ShellMessage::Signal { signal: 9 });
        let last = next();
        assert_eq!(last.len(), 1, "a killed read sent more: {last:?}");
        assert!(last[0].1 & FLAG_TERMINAL != 0);
        assert!(matches!(&last[0].2, ShellMessage::FsError { code, .. } if code == "S218"));
    }

    /// Drive the dispatcher over an in-process socketpair and assert
    /// the happy path for a pipe-mode `echo` invocation.
    ///
    /// Rationale: we can't attach the socketpair to a `File` handle
    /// easily (the `run` entry point opens the path itself), so we
    /// duplicate just enough of the dispatch glue here to prove the
    /// message-handling logic is correct. Integration coverage for
    /// the full virtio-console path lives in sandbox-builtin's VM
    /// tests.
    fn run_over_socketpair() {
        ensure_test_reaper();
        let (host, guest) = UnixStream::pair().expect("socketpair");
        host.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        // Guest thread: replicate the dispatcher's core loop against
        // the socketpair ends directly, bypassing File::open.
        let guest_clone = guest.try_clone().unwrap();
        thread::spawn(move || {
            let guest_file = unsafe {
                use std::os::unix::io::{FromRawFd, IntoRawFd};
                File::from_raw_fd(guest.into_raw_fd())
            };
            let writer = spawn_writer_thread(guest_file);
            let sessions: SessionRegistry = Arc::new(Mutex::new(HashMap::new()));
            let fs_writes: FsWriteRegistry = Arc::new(Mutex::new(HashMap::new()));
            let windows = Windows::default();
            let mut reader = BufReader::new(guest_clone);
            while let Ok(Some((corr_id, _f, msg))) = sp::read_frame_blocking(&mut reader) {
                handle_frame(corr_id, msg, &sessions, &writer, &fs_writes, &windows);
            }
        });

        // Host: send a Request for `echo hi`, drive stdin EOF, collect
        // Started + Stdout + Exited.
        let req = ShellMessage::Request {
            cmd: "/bin/sh".into(),
            args: vec!["-c".into(), "echo hi".into()],
            env: vec![],
            cwd: None,
            tty: false,
            rows: 24,
            cols: 80,
        };
        let mut host_writer = host.try_clone().unwrap();
        sp::write_frame_blocking(&mut host_writer, 1, 0, &req).unwrap();
        sp::write_frame_blocking(
            &mut host_writer,
            1,
            0,
            &ShellMessage::Stdin {
                data_b64: String::new(),
            },
        )
        .unwrap();

        let mut host_reader = BufReader::new(host);
        let mut saw_started = false;
        let mut got_stdout = Vec::new();
        let mut exited_code: Option<i32> = None;
        while exited_code.is_none() {
            let (corr_id, flags, msg) = sp::read_frame_blocking(&mut host_reader)
                .expect("read frame")
                .expect("no EOF before Exited");
            assert_eq!(corr_id, 1);
            match msg {
                ShellMessage::Started { .. } => saw_started = true,
                ShellMessage::Stdout { data_b64 } => {
                    got_stdout.extend(decode_b64(&data_b64).unwrap());
                }
                ShellMessage::Stderr { .. } => {}
                ShellMessage::Exited { code } => {
                    assert!(flags & FLAG_TERMINAL != 0, "Exited missing terminal flag");
                    exited_code = Some(code);
                }
                // The guest took our stdin frames.
                ShellMessage::Credit { .. } => {}
                other => panic!("unexpected message: {other:?}"),
            }
        }
        assert!(saw_started);
        assert_eq!(exited_code, Some(0));
        assert_eq!(String::from_utf8_lossy(&got_stdout), "hi\n");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn echo_pipe_session_happy_path() {
        // Some CI environments don't have /bin/sh; guard with a
        // pre-check so the test skips cleanly rather than flakes.
        if skip_if_missing("/bin/sh") {
            return;
        }
        let _g = serial_spawn_guard();
        run_over_socketpair();
    }

    /// TTY mode happy path: `/bin/sh -c 'echo hi'` should succeed
    /// and produce at least "hi" somewhere in its output. The PTY
    /// line discipline may transform line endings (\n -> \r\n) and
    /// shells can emit prompts or other chatter, so we assert a
    /// containment rather than exact equality.
    #[test]
    #[cfg(target_os = "linux")]
    fn tty_session_happy_path() {
        if skip_if_missing("/bin/sh") {
            return;
        }
        let _g = serial_spawn_guard();
        ensure_test_reaper();
        let (host, guest) = UnixStream::pair().expect("socketpair");
        host.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        let guest_clone = guest.try_clone().unwrap();
        thread::spawn(move || {
            let guest_file = unsafe {
                use std::os::unix::io::{FromRawFd, IntoRawFd};
                File::from_raw_fd(guest.into_raw_fd())
            };
            let writer = spawn_writer_thread(guest_file);
            let sessions: SessionRegistry = Arc::new(Mutex::new(HashMap::new()));
            let fs_writes: FsWriteRegistry = Arc::new(Mutex::new(HashMap::new()));
            let windows = Windows::default();
            let mut reader = BufReader::new(guest_clone);
            while let Ok(Some((corr_id, _f, msg))) = sp::read_frame_blocking(&mut reader) {
                handle_frame(corr_id, msg, &sessions, &writer, &fs_writes, &windows);
            }
        });

        let req = ShellMessage::Request {
            cmd: "/bin/sh".into(),
            args: vec!["-c".into(), "echo hi".into()],
            env: vec![],
            cwd: None,
            tty: true,
            rows: 24,
            cols: 80,
        };
        let mut host_writer = host.try_clone().unwrap();
        sp::write_frame_blocking(&mut host_writer, 7, 0, &req).unwrap();

        let mut host_reader = BufReader::new(host);
        let mut saw_started = false;
        let mut output = Vec::new();
        let mut exited_code: Option<i32> = None;
        while exited_code.is_none() {
            let (corr_id, flags, msg) = sp::read_frame_blocking(&mut host_reader)
                .expect("read frame")
                .expect("no EOF before Exited");
            assert_eq!(corr_id, 7);
            match msg {
                ShellMessage::Started { .. } => saw_started = true,
                ShellMessage::Stdout { data_b64 } => {
                    output.extend(decode_b64(&data_b64).unwrap());
                }
                ShellMessage::Exited { code } => {
                    assert!(flags & FLAG_TERMINAL != 0);
                    exited_code = Some(code);
                }
                other => panic!("unexpected message: {other:?}"),
            }
        }
        assert!(saw_started);
        assert_eq!(exited_code, Some(0));
        let output_str = String::from_utf8_lossy(&output);
        assert!(
            output_str.contains("hi"),
            "expected 'hi' in PTY output, got: {output_str:?}"
        );
    }

    #[test]
    fn b64_roundtrip() {
        let raw = b"hello, world\n\0\xff";
        let encoded = encode_b64(raw);
        let decoded = decode_b64(&encoded).unwrap();
        assert_eq!(decoded, raw);
    }

    // ----- Integration tests per plan spec -------------------------
    //
    // The plan (~/.claude/plans/use-graphify-to-query-parallel-naur.md,
    // Verification §Integration tests) lists six scenarios we need to
    // cover end-to-end. We drive them here against an in-process
    // socketpair rather than a live libkrun VM: the dispatcher is the
    // only component whose behavior is specific to this branch, and
    // all six plan cases exercise dispatcher semantics (spawn, I/O
    // routing, signal delivery, concurrency, exit-code encoding).
    // Full VM-level coverage belongs behind the `integration-vm`
    // feature flag in `sandbox-builtin` and is tracked as follow-up.

    /// Boot the dispatcher glue over a fresh socketpair. Returns a
    /// blocking host-side reader and a writable host-side sender the
    /// test can use to drive requests. The guest loop lives in a
    /// detached thread for the lifetime of the socketpair.
    #[cfg(target_os = "linux")]
    fn start_dispatcher_over_socketpair() -> (UnixStream, UnixStream) {
        ensure_test_reaper();
        let (host, guest) = UnixStream::pair().expect("socketpair");
        host.set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();

        let guest_clone = guest.try_clone().unwrap();
        thread::spawn(move || {
            let guest_file = unsafe {
                use std::os::unix::io::{FromRawFd, IntoRawFd};
                File::from_raw_fd(guest.into_raw_fd())
            };
            let writer = spawn_writer_thread(guest_file);
            let sessions: SessionRegistry = Arc::new(Mutex::new(HashMap::new()));
            let fs_writes: FsWriteRegistry = Arc::new(Mutex::new(HashMap::new()));
            let windows = Windows::default();
            let mut reader = BufReader::new(guest_clone);
            while let Ok(Some((corr_id, _f, msg))) = sp::read_frame_blocking(&mut reader) {
                handle_frame(corr_id, msg, &sessions, &writer, &fs_writes, &windows);
            }
        });

        let host_write = host.try_clone().unwrap();
        (host, host_write)
    }

    /// Drive a single pipe-mode session to completion. Collects
    /// stdout, stderr, and the exit code; returns a tuple so the
    /// caller can assert. Panics on timeout or protocol error so
    /// test failures point at the actual stuck frame.
    #[cfg(target_os = "linux")]
    fn drive_pipe_session(
        host_read: UnixStream,
        host_write: &mut UnixStream,
        corr_id: u32,
        req: ShellMessage,
        stdin_bytes: Option<&[u8]>,
    ) -> (Vec<u8>, Vec<u8>, i32) {
        sp::write_frame_blocking(host_write, corr_id, 0, &req).unwrap();
        if let Some(bytes) = stdin_bytes {
            sp::write_frame_blocking(
                host_write,
                corr_id,
                0,
                &ShellMessage::Stdin {
                    data_b64: encode_b64(bytes),
                },
            )
            .unwrap();
        }
        // Send stdin EOF so programs like `cat` exit.
        sp::write_frame_blocking(
            host_write,
            corr_id,
            0,
            &ShellMessage::Stdin {
                data_b64: String::new(),
            },
        )
        .unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_code: Option<i32> = None;
        let mut host_reader = BufReader::new(host_read);
        while exit_code.is_none() {
            let (got_id, flags, msg) = sp::read_frame_blocking(&mut host_reader)
                .expect("read frame")
                .expect("EOF before Exited");
            assert_eq!(got_id, corr_id, "frame for wrong corr_id");
            match msg {
                ShellMessage::Started { .. } => {}
                ShellMessage::Stdout { data_b64 } => {
                    stdout.extend(decode_b64(&data_b64).unwrap());
                }
                ShellMessage::Stderr { data_b64 } => {
                    stderr.extend(decode_b64(&data_b64).unwrap());
                }
                ShellMessage::Error { message } => {
                    panic!("guest returned Error: {message}");
                }
                ShellMessage::Exited { code } => {
                    assert!(flags & FLAG_TERMINAL != 0, "Exited without terminal flag");
                    exit_code = Some(code);
                }
                // The guest took our stdin frames.
                ShellMessage::Credit { .. } => {}
                other => panic!("unexpected inbound: {other:?}"),
            }
        }
        (stdout, stderr, exit_code.unwrap())
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn plan_exit_code_propagates() {
        if skip_if_missing("/bin/sh") {
            return;
        }
        let _g = serial_spawn_guard();
        let (host_read, mut host_write) = start_dispatcher_over_socketpair();
        let (_out, _err, code) = drive_pipe_session(
            host_read,
            &mut host_write,
            1,
            ShellMessage::Request {
                cmd: "/bin/sh".into(),
                args: vec!["-c".into(), "exit 42".into()],
                env: vec![],
                cwd: None,
                tty: false,
                rows: 24,
                cols: 80,
            },
            None,
        );
        assert_eq!(code, 42, "shell `exit 42` must propagate");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn plan_stderr_is_separated_from_stdout() {
        if skip_if_missing("/bin/sh") {
            return;
        }
        let _g = serial_spawn_guard();
        let (host_read, mut host_write) = start_dispatcher_over_socketpair();
        let (out, err, code) = drive_pipe_session(
            host_read,
            &mut host_write,
            2,
            ShellMessage::Request {
                cmd: "/bin/sh".into(),
                args: vec!["-c".into(), "echo err >&2".into()],
                env: vec![],
                cwd: None,
                tty: false,
                rows: 24,
                cols: 80,
            },
            None,
        );
        assert_eq!(code, 0);
        assert!(out.is_empty(), "stdout should be empty, got {out:?}");
        assert_eq!(String::from_utf8_lossy(&err), "err\n");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn plan_stdin_pipe_round_trip() {
        if skip_if_missing("/bin/cat") {
            return;
        }
        let _g = serial_spawn_guard();
        let (host_read, mut host_write) = start_dispatcher_over_socketpair();
        let payload = b"roundtrip payload\n";
        let (out, _err, code) = drive_pipe_session(
            host_read,
            &mut host_write,
            3,
            ShellMessage::Request {
                cmd: "/bin/cat".into(),
                args: vec![],
                env: vec![],
                cwd: None,
                tty: false,
                rows: 24,
                cols: 80,
            },
            Some(payload),
        );
        assert_eq!(code, 0);
        assert_eq!(out, payload, "cat must echo stdin back to stdout");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn plan_four_concurrent_sessions_complete_independently() {
        // Four parallel `sleep 0.1` across four distinct corr_ids on
        // one dispatcher. Each must emit Started + Exited without
        // cross-talk between ids.
        if skip_if_missing("/bin/sleep") {
            return;
        }
        let _g = serial_spawn_guard();
        ensure_test_reaper();
        let (host, _) = UnixStream::pair().expect("socketpair");
        let (_writer_unused, _) = {
            // Reuse the integration helper but we need one shared
            // guest + one shared host for multiplexing. Rebuild
            // manually here.
            let (h, g) = UnixStream::pair().expect("socketpair");
            h.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let g_clone = g.try_clone().unwrap();
            thread::spawn(move || {
                let f = unsafe {
                    use std::os::unix::io::{FromRawFd, IntoRawFd};
                    File::from_raw_fd(g.into_raw_fd())
                };
                let w = spawn_writer_thread(f);
                let sessions: SessionRegistry = Arc::new(Mutex::new(HashMap::new()));
                let fs_writes: FsWriteRegistry = Arc::new(Mutex::new(HashMap::new()));
                let windows = Windows::default();
                let mut r = BufReader::new(g_clone);
                while let Ok(Some((id, _f, msg))) = sp::read_frame_blocking(&mut r) {
                    handle_frame(id, msg, &sessions, &w, &fs_writes, &windows);
                }
            });
            (h, 0u8)
        };
        // Kill the first unused pair; keep the second alive.
        drop(host);
        let (host, g) = UnixStream::pair().expect("socketpair");
        host.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let g_clone = g.try_clone().unwrap();
        thread::spawn(move || {
            let f = unsafe {
                use std::os::unix::io::{FromRawFd, IntoRawFd};
                File::from_raw_fd(g.into_raw_fd())
            };
            let w = spawn_writer_thread(f);
            let sessions: SessionRegistry = Arc::new(Mutex::new(HashMap::new()));
            let fs_writes: FsWriteRegistry = Arc::new(Mutex::new(HashMap::new()));
            let windows = Windows::default();
            let mut r = BufReader::new(g_clone);
            while let Ok(Some((id, _f, msg))) = sp::read_frame_blocking(&mut r) {
                handle_frame(id, msg, &sessions, &w, &fs_writes, &windows);
            }
        });

        let mut host_writer = host.try_clone().unwrap();
        for id in 1..=4u32 {
            sp::write_frame_blocking(
                &mut host_writer,
                id,
                0,
                &ShellMessage::Request {
                    cmd: "/bin/sleep".into(),
                    args: vec!["0.1".into()],
                    env: vec![],
                    cwd: None,
                    tty: false,
                    rows: 24,
                    cols: 80,
                },
            )
            .unwrap();
        }

        let mut reader = BufReader::new(host);
        let mut exited: std::collections::HashSet<u32> = std::collections::HashSet::new();
        while exited.len() < 4 {
            let (id, flags, msg) = sp::read_frame_blocking(&mut reader)
                .expect("read frame")
                .expect("EOF before all Exited");
            match msg {
                ShellMessage::Exited { code } => {
                    assert_eq!(code, 0, "sleep should exit 0 for corr_id={id}");
                    assert!(flags & FLAG_TERMINAL != 0);
                    exited.insert(id);
                }
                ShellMessage::Started { .. }
                | ShellMessage::Stdout { .. }
                | ShellMessage::Stderr { .. } => {}
                other => panic!("unexpected: {other:?}"),
            }
        }
        assert_eq!(exited, [1u32, 2, 3, 4].iter().copied().collect());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn plan_sigkill_signal_terminates_child() {
        // Send `Signal { signal: 9 }` at a sleeping child. Exit code
        // follows the shell convention `128 + signal`.
        if skip_if_missing("/bin/sleep") {
            return;
        }
        let _g = serial_spawn_guard();
        let (host_read, mut host_write) = start_dispatcher_over_socketpair();
        let corr_id = 42u32;
        sp::write_frame_blocking(
            &mut host_write,
            corr_id,
            0,
            &ShellMessage::Request {
                cmd: "/bin/sleep".into(),
                args: vec!["30".into()],
                env: vec![],
                cwd: None,
                tty: false,
                rows: 24,
                cols: 80,
            },
        )
        .unwrap();

        // Wait for Started so the child PID exists before signaling.
        let mut reader = BufReader::new(host_read);
        loop {
            let (id, _f, msg) = sp::read_frame_blocking(&mut reader)
                .expect("read")
                .expect("EOF");
            assert_eq!(id, corr_id);
            if matches!(msg, ShellMessage::Started { .. }) {
                break;
            }
        }

        sp::write_frame_blocking(
            &mut host_write,
            corr_id,
            0,
            &ShellMessage::Signal { signal: 9 },
        )
        .unwrap();

        loop {
            let (id, flags, msg) = sp::read_frame_blocking(&mut reader)
                .expect("read")
                .expect("EOF before Exited");
            assert_eq!(id, corr_id);
            if let ShellMessage::Exited { code } = msg {
                assert!(flags & FLAG_TERMINAL != 0);
                assert_eq!(code, 128 + 9, "SIGKILL → 128 + 9 = 137, got {code}");
                break;
            }
        }
    }

    // ----- Flow control, both ways ---------------------------------
    //
    // `SimHost` plays relay and client on one socketpair: each session
    // takes its own frames, grants output Credit for each one it takes,
    // and sends input at most a window ahead of the guest's Credit. Any
    // frame for a corr_id that already ended, or never started, is a leak.

    #[cfg(target_os = "linux")]
    type Route = std::sync::mpsc::Sender<(u8, ShellMessage)>;

    #[cfg(target_os = "linux")]
    struct SimHost {
        out: Arc<Mutex<UnixStream>>,
        routes: Arc<Mutex<HashMap<u32, Route>>>,
        leaks: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[cfg(target_os = "linux")]
    impl SimHost {
        fn start() -> Self {
            let (host_read, host_write) = start_dispatcher_over_socketpair();
            // A guest that stops reading fails the test instead of hanging it.
            host_write
                .set_write_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            let routes: Arc<Mutex<HashMap<u32, Route>>> = Arc::default();
            let leaks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let (routes_in, leaks_in) = (routes.clone(), leaks.clone());
            thread::spawn(move || {
                host_read.set_read_timeout(None).unwrap();
                let mut reader = BufReader::new(host_read);
                while let Ok(Some((id, flags, msg))) = sp::read_frame_blocking(&mut reader) {
                    let mut routes = routes_in.lock().unwrap();
                    let Some(route) = routes.get(&id) else {
                        leaks_in.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        eprintln!("leak: corr_id={id} {msg:?}");
                        continue;
                    };
                    let _ = route.send((flags, msg));
                    if flags & FLAG_TERMINAL != 0 {
                        routes.remove(&id);
                    }
                }
            });
            Self {
                out: Arc::new(Mutex::new(host_write)),
                routes,
                leaks,
            }
        }

        fn open(&self, id: u32, first: ShellMessage) -> SimSession {
            let (tx, rx) = std::sync::mpsc::channel();
            self.routes.lock().unwrap().insert(id, tx);
            let session = SimSession {
                id,
                rx,
                out: self.out.clone(),
                credit: sp::SEND_WINDOW,
                data: Vec::new(),
                end: None,
                delay: Duration::ZERO,
            };
            session.send(first);
            session
        }

        /// Runs `cmd` with `stdin` (then EOF); its stdout and exit code.
        fn run(&self, id: u32, cmd: &str, stdin: &[u8]) -> (Vec<u8>, ShellMessage) {
            let mut s = self.open(id, sh(cmd));
            for chunk in stdin.chunks(64 * 1024).chain([&[][..]]) {
                s.send_input(ShellMessage::Stdin {
                    data_b64: encode_b64(chunk),
                });
            }
            s.wait()
        }

        fn upload(&self, id: u32, path: &Path, data: &[u8]) -> ShellMessage {
            let mut s = self.open(
                id,
                ShellMessage::FsRequest(iii_shell_proto::FsOp::WriteStart {
                    path: path.to_string_lossy().into_owned(),
                    mode: "0644".into(),
                    parents: true,
                }),
            );
            for chunk in data.chunks(64 * 1024) {
                s.send_input(ShellMessage::FsChunk {
                    data_b64: encode_b64(chunk),
                });
            }
            s.send_input(ShellMessage::FsEnd);
            s.wait().1
        }

        fn download(&self, id: u32, path: &Path) -> (Vec<u8>, ShellMessage) {
            let path = path.to_string_lossy().into_owned();
            let read = iii_shell_proto::FsOp::ReadStart { path };
            self.open(id, ShellMessage::FsRequest(read)).wait()
        }

        fn leaks(&self) -> usize {
            self.leaks.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[cfg(target_os = "linux")]
    struct SimSession {
        id: u32,
        rx: std::sync::mpsc::Receiver<(u8, ShellMessage)>,
        out: Arc<Mutex<UnixStream>>,
        /// Input frames the guest still takes.
        credit: u32,
        /// Output bytes so far.
        data: Vec<u8>,
        /// The terminal frame, once it came.
        end: Option<ShellMessage>,
        /// How long the consumer takes over each frame.
        delay: Duration,
    }

    #[cfg(target_os = "linux")]
    impl SimSession {
        fn send(&self, msg: ShellMessage) {
            let mut out = self.out.lock().unwrap();
            sp::write_frame_blocking(&mut *out, self.id, 0, &msg).unwrap();
        }

        /// Takes one frame. A stall this long is a deadlock.
        fn pump(&mut self) {
            thread::sleep(self.delay);
            let (flags, msg) = self
                .rx
                .recv_timeout(Duration::from_secs(30))
                .unwrap_or_else(|_| panic!("corr_id={} stalled", self.id));
            match msg {
                ShellMessage::Credit { frames } => {
                    assert!(flags & FLAG_CONTROL != 0, "input credit must be control");
                    self.credit += frames;
                }
                ShellMessage::Stdout { data_b64 }
                | ShellMessage::Stderr { data_b64 }
                | ShellMessage::FsChunk { data_b64 } => {
                    self.data.extend(decode_b64(&data_b64).unwrap());
                    self.send(ShellMessage::Credit { frames: 1 });
                }
                ShellMessage::Started { .. } | ShellMessage::FsMeta(_) => {}
                end => {
                    assert!(flags & FLAG_TERMINAL != 0, "not an end: {end:?}");
                    self.end = Some(end);
                }
            }
        }

        fn send_input(&mut self, msg: ShellMessage) {
            while self.credit == 0 {
                self.pump();
                assert!(self.end.is_none(), "ended mid-input: {:?}", self.end);
            }
            self.send(msg);
            self.credit -= 1;
        }

        fn wait(mut self) -> (Vec<u8>, ShellMessage) {
            while self.end.is_none() {
                self.pump();
            }
            (self.data, self.end.unwrap())
        }
    }

    #[cfg(target_os = "linux")]
    fn sh(cmd: &str) -> ShellMessage {
        ShellMessage::Request {
            cmd: "/bin/sh".into(),
            args: vec!["-c".into(), cmd.into()],
            env: vec![],
            cwd: None,
            tty: false,
            rows: 24,
            cols: 80,
        }
    }

    #[cfg(target_os = "linux")]
    fn pattern(len: usize, seed: usize) -> Vec<u8> {
        (0..len)
            .map(|i| ((i / 7 + i + seed * 31) % 251) as u8)
            .collect()
    }

    #[cfg(target_os = "linux")]
    const EXITED_0: ShellMessage = ShellMessage::Exited { code: 0 };

    /// 128 MiB through `cat`. The dispatcher once wrote stdin itself: with
    /// `cat` blocked on its output, that write never returned, and the
    /// Credit that would unblock the output sat unread behind it.
    #[test]
    #[cfg(target_os = "linux")]
    fn a_child_echoing_a_large_stdin_finishes() {
        let _g = serial_spawn_guard();
        let host = SimHost::start();
        let data = pattern(128 << 20, 1);
        let (out, end) = host.run(1, "cat", &data);
        assert_eq!((out.len(), end), (data.len(), EXITED_0));
        assert!(out == data, "the echo differs from its stdin");
        assert_eq!(host.leaks(), 0);
    }

    /// A child that never reads its stdin holds back only its own session:
    /// another exec runs meanwhile, and the SIGKILL still reaches it.
    #[test]
    #[cfg(target_os = "linux")]
    fn a_child_that_reads_no_stdin_holds_back_only_its_session() {
        let _g = serial_spawn_guard();
        let host = SimHost::start();
        let mut stuck = host.open(1, sh("sleep 30"));
        // A window of input: more than the child's pipe holds.
        for _ in 0..sp::SEND_WINDOW {
            stuck.send_input(ShellMessage::Stdin {
                data_b64: encode_b64(&[7u8; 64 * 1024]),
            });
        }
        let (out, end) = host.run(2, "echo hi", &[]);
        assert_eq!((out, end), (b"hi\n".to_vec(), EXITED_0));
        stuck.send(ShellMessage::Signal { signal: 9 });
        assert_eq!(stuck.wait().1, ShellMessage::Exited { code: 137 });
        assert_eq!(host.leaks(), 0);
    }

    /// Input past the window is never dropped: the session is killed.
    #[test]
    #[cfg(target_os = "linux")]
    fn stdin_past_its_window_kills_its_session() {
        let _g = serial_spawn_guard();
        let host = SimHost::start();
        let flood = host.open(1, sh("sleep 30"));
        for _ in 0..3 * sp::SEND_WINDOW {
            flood.send(ShellMessage::Stdin {
                data_b64: encode_b64(&[7u8; 64 * 1024]),
            });
        }
        assert_eq!(flood.wait().1, ShellMessage::Exited { code: 137 });
    }

    /// A consumer that takes its output slowly holds back only its own
    /// session, which still arrives whole.
    #[test]
    #[cfg(target_os = "linux")]
    fn a_slow_consumer_holds_back_only_its_own_session() {
        let _g = serial_spawn_guard();
        let host = Arc::new(SimHost::start());
        let slow_host = host.clone();
        let slow = thread::spawn(move || {
            let data = pattern(4 << 20, 2);
            let mut s = slow_host.open(1, sh("cat"));
            s.delay = Duration::from_millis(20);
            for chunk in data.chunks(64 * 1024).chain([&[][..]]) {
                s.send_input(ShellMessage::Stdin {
                    data_b64: encode_b64(chunk),
                });
            }
            let (out, end) = s.wait();
            assert_eq!(end, EXITED_0);
            assert!(out == data, "the slow echo differs");
            std::time::Instant::now()
        });
        thread::sleep(Duration::from_millis(100));
        let data = pattern(8 << 20, 3);
        let (out, end) = host.run(2, "cat", &data);
        let fast_done = std::time::Instant::now();
        assert_eq!(end, EXITED_0);
        assert!(out == data, "the fast echo differs");
        assert!(fast_done < slow.join().unwrap(), "the fast session waited");
        assert_eq!(host.leaks(), 0);
    }

    /// An echo killed mid-stream ends with 137, and nothing of it follows
    /// its end: no stdout, no input credit from its stdin writer, which a
    /// process outside the killed group holds up until after that end.
    #[test]
    #[cfg(target_os = "linux")]
    fn a_killed_echo_sends_nothing_after_its_end() {
        if skip_if_missing("/usr/bin/setsid") {
            return;
        }
        let _g = serial_spawn_guard();
        let host = SimHost::start();
        // fd 3: sh gives a background job /dev/null for stdin.
        let holder = "exec 3<&0; setsid sleep 2 <&3 >/dev/null 2>&1 &";
        let s = host.open(1, sh(&format!("{holder} exec cat")));
        let chunk = encode_b64(&pattern(64 * 1024, 4));
        // A window of input and no output taken: `cat` stops on its full
        // stdout, so most of the window is still queued when the kill lands.
        for _ in 0..sp::SEND_WINDOW {
            s.send(ShellMessage::Stdin {
                data_b64: chunk.clone(),
            });
        }
        thread::sleep(Duration::from_millis(300));
        s.send(ShellMessage::Signal { signal: 9 });
        assert_eq!(s.wait().1, ShellMessage::Exited { code: 137 });
        // Past the holder's exit, which frees the stdin writer.
        thread::sleep(Duration::from_millis(2500));
        assert_eq!(host.leaks(), 0, "frames after the session's end");
    }

    /// 128 MiB uploaded within its window lands whole; the guest's write
    /// queue once held 64 chunks and aborted the upload past them (S218).
    #[test]
    #[cfg(target_os = "linux")]
    fn a_large_upload_lands_whole() {
        let _g = serial_spawn_guard();
        let host = SimHost::start();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        let data = pattern(128 << 20, 5);
        let reply = host.upload(1, &path, &data);
        assert!(
            matches!(reply, ShellMessage::FsResponse(iii_shell_proto::FsResult::Write { bytes_written, .. }) if bytes_written == data.len() as u64),
            "{reply:?}"
        );
        assert!(std::fs::read(&path).unwrap() == data, "the file differs");
        assert_eq!(host.leaks(), 0);
    }

    /// 30 sessions at once in both directions: echoes, uploads and reads
    /// all arrive whole, each on its own corr_id.
    #[test]
    #[cfg(target_os = "linux")]
    fn thirty_sessions_both_ways_arrive_whole() {
        let _g = serial_spawn_guard();
        let host = Arc::new(SimHost::start());
        let dir = Arc::new(tempfile::tempdir().unwrap());
        for i in 0..10 {
            std::fs::write(dir.path().join(format!("r{i}")), pattern(2 << 20, i)).unwrap();
        }
        let sessions: Vec<_> = (0..30u32)
            .map(|id| {
                let (host, dir) = (host.clone(), dir.clone());
                thread::spawn(move || {
                    let i = id as usize / 3;
                    let data = pattern(2 << 20, i);
                    match id % 3 {
                        0 => {
                            let (out, end) = host.run(id + 1, "cat", &data);
                            assert_eq!(end, EXITED_0);
                            assert!(out == data, "echo {id} differs");
                        }
                        1 => {
                            let path = dir.path().join(format!("w{i}"));
                            let reply = host.upload(id + 1, &path, &data);
                            assert!(matches!(reply, ShellMessage::FsResponse(_)), "{reply:?}");
                            assert!(std::fs::read(&path).unwrap() == data, "upload {id} differs");
                        }
                        _ => {
                            let path = dir.path().join(format!("r{i}"));
                            let (out, end) = host.download(id + 1, &path);
                            assert_eq!(end, ShellMessage::FsEnd);
                            assert!(out == data, "read {id} differs");
                        }
                    }
                })
            })
            .collect();
        for session in sessions {
            session.join().unwrap();
        }
        assert_eq!(host.leaks(), 0);
    }
}
