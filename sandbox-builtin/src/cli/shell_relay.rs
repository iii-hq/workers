// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Host-side multiplexing relay for the `iii.exec` virtio-console
//! channel.
//!
//! Sits in the `__vm-boot` process (same lifetime as the VM) and
//! bridges client connections on a Unix socket with the host end of a
//! socketpair that's wired through libkrun into the guest. The relay
//! fans out multiple concurrent exec sessions on the same port by
//! routing frames on their `corr_id` (see `iii_supervisor::shell_protocol`).
//!
//! Design: the relay does **not** parse JSON payloads, only the
//! fixed-size frame header.
//! This keeps routing cheap and isolates the relay from protocol
//! additions — new `ShellMessage` variants don't touch this file.
//!
//! Concurrency: `vm_writer_task` is the single consumer of a
//! bounded mpsc. `routes` and each client's `owned_ids` are
//! `std::sync::Mutex`-guarded and locked on every inbound frame;
//! critical sections are small. Not lock-free end-to-end — if the
//! workload ever grows a "many clients × high message rate" axis,
//! `DashMap` / `parking_lot::RwLock` would be drop-in replacements.
//!
//! Handshake: when a client connects, the relay sends a 4-byte
//! big-endian `id_offset` announcing which `corr_id` range it owns.
//! The client generates ids starting at `id_offset + 1` and stays
//! below `id_offset + ID_RANGE_STEP`. Every connection gets a range
//! no live client holds and no session the guest may still be ending
//! uses (see [`Clients`]), so a session's late frames can never reach
//! another client, and a client's slot is free again the moment it
//! leaves.
//!
//! Flow control: the guest sends a session at most
//! [`iii_shell_proto::SEND_WINDOW`] output frames ahead of what its
//! client has read, and the relay grants more with a `Credit` frame as
//! it writes them to the client. So `vm_reader_task` never waits for a
//! client: a slow reader holds back only its own session, and a large
//! `fs::read` arrives whole however many other sessions are live.
//!
//! Authorization: connecting peers must share our euid (checked via
//! `SO_PEERCRED` on Linux, `getpeereid` on macOS/BSD). Exec is a
//! local-owner affordance, not a network service.

use std::collections::{HashMap, HashSet};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

use iii_supervisor::shell_protocol::{
    FRAME_HEADER_SIZE, MAX_FRAME_SIZE, SEND_WINDOW, ShellMessage, encode_frame,
    flags::FLAG_TERMINAL,
};

/// Maximum concurrent clients. Each gets a non-overlapping slice of
/// the `u32` `corr_id` space so neither side needs a central id
/// allocator. Rare for more than one or two to be active at a time
/// (interactive dev sessions), but headroom is cheap.
pub const MAX_CLIENTS: u32 = 16;

/// Size of each connection's `corr_id` range. Ranges cycle through the
/// `u32` space, one per connection.
pub const ID_RANGE_STEP: u32 = 1 << 16;

/// A pre-read frame ready to forward. Owns its bytes — tasks hand
/// these around via mpsc and write them out whole.
type FrameBytes = Vec<u8>;

/// Per-client bytes-in-flight cap for the shared VM write queue.
/// Bounds aggregate relay memory at `MAX_CLIENTS × cap = 1 GiB` —
/// without it, the frame-count bound on `vm_write_tx` lets one
/// client pin ~4 GiB with max-sized frames.
const PER_CLIENT_VM_BYTES_CAP: u64 = 64 * 1024 * 1024;

/// A frame waiting in the shared client→VM queue. The optional
/// [`ClientBytesGuard`]'s `Drop` decrements the owning client's
/// byte counter when `vm_writer_task` consumes the frame. `None`
/// for internal frames (e.g. SIGKILL fan-out on disconnect).
struct QueuedFrame {
    bytes: FrameBytes,
    _guard: Option<ClientBytesGuard>,
}

struct ClientBytesGuard {
    counter: Arc<AtomicU64>,
    size: u64,
}

impl Drop for ClientBytesGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(self.size, Ordering::Release);
    }
}

/// Route entry: a frame sender paired with a back-pointer to the
/// owning client's `owned_ids` set. The back-pointer lets
/// `vm_reader_task`'s terminal-frame handler decrement the client's
/// owned-ids count when a session completes — without it, a client
/// that legitimately runs [`MAX_OWNED_IDS_PER_CLIENT`] sessions to
/// completion would stay locked at the cap forever.
struct RouteEntry {
    sender: mpsc::Sender<FrameBytes>,
    owner_ids: Arc<Mutex<HashSet<u32>>>,
    /// Ends the owning client's session; see [`CLIENT_WRITE_CAPACITY`].
    kick: Arc<tokio::sync::Notify>,
    /// A frame of this session was dropped, so none after it may be
    /// delivered. The entry stays until the terminal frame.
    overrun: bool,
}

/// Maps a `corr_id` to the [`RouteEntry`] that delivers its response
/// frames. Populated when the relay sees the first frame for a new id
/// from the client direction; pruned when a terminal frame crosses
/// through or the owning client disconnects.
type Routes = Arc<Mutex<HashMap<u32, RouteEntry>>>;

/// Live connections and the `corr_id` ranges handed out to them.
#[derive(Default)]
struct Clients {
    /// The `id_offset` of every live connection; at most [`MAX_CLIENTS`].
    live: HashSet<u32>,
    /// Where the search for the next free range starts.
    next_range: u32,
    /// Sessions whose client left before the guest ended them. Their
    /// late frames find no route, and their range is not handed out
    /// again until the guest's terminal frame for them arrives.
    lingering: HashSet<u32>,
}

type ClientTable = Arc<Mutex<Clients>>;

/// How long a client over [`MAX_CLIENTS`] waits for another to leave
/// before it is refused, so a burst of timed-out execs does not refuse
/// the next ones.
const SLOT_WAIT: Duration = Duration::from_millis(500);

/// Bound on the client → VM channel feeding `vm_writer_task`. Every
/// frame from every client funnels through this queue in arrival
/// order. The bound backpressures individual client readers (via
/// `Sender::send().await`) rather than letting a misbehaving client
/// balloon relay memory unboundedly. 1024 frames ≈ 4 MiB at max frame
/// size — plenty of room for bursty stdin without runaway growth.
const VM_WRITE_CAPACITY: usize = 1024;

/// Bound on each VM → client channel. Flow control keeps a session
/// within [`SEND_WINDOW`] frames plus a few control frames, so only a
/// guest that ignores its credit fills it. `vm_reader_task` then fails
/// that session alone: it drops its frames from there on and
/// disconnects its client, whose read or exec reports the cut.
const CLIENT_WRITE_CAPACITY: usize = 1024;

/// Delivered frames of a session per `Credit` frame back to the guest;
/// below [`SEND_WINDOW`], so the guest never waits on a batch.
const CREDIT_BATCH: u32 = SEND_WINDOW / 4;

/// How long a client may take no frame at all before its session is
/// treated as abandoned and ended (its guest side SIGKILLed): a caller that
/// never reads an `fs::read` channel must not hold a relay slot and a guest
/// thread until the sandbox is reaped. A slow reader still takes a frame
/// far more often than this.
#[cfg(not(test))]
const CLIENT_STALL: Duration = Duration::from_secs(30);
#[cfg(test)]
const CLIENT_STALL: Duration = Duration::from_secs(1);

/// Maximum number of distinct `corr_id`s a single client can register
/// routes for concurrently. Each registered id pins an entry in the
/// shared `routes` map plus a `Vec` slot in the client's `owned_ids`.
/// Without a cap, a same-uid peer can flood distinct ids (up to
/// `ID_RANGE_STEP` ≈ 268M) to balloon relay memory — roughly 6 GiB per
/// saturated slot. 256 concurrent sessions is already more than any
/// realistic interactive workload; real clients never come close.
const MAX_OWNED_IDS_PER_CLIENT: usize = 256;

/// Identity of a bound Unix socket, recorded at bind time so a later
/// cleanup can refuse to unlink the path if it's been replaced.
#[derive(Debug, Clone)]
pub struct ShellSocketFingerprint {
    /// Absolute path we bound.
    pub path: String,
    dev: u64,
    ino: u64,
}

impl ShellSocketFingerprint {
    /// Unlink the socket file iff its (dev, ino) still match the ones
    /// recorded at bind time. A fast `stop → start` cycle can let a
    /// fresh VM rebind the path under us; without this check, stale
    /// cleanup would silently delete the live socket.
    pub fn remove_if_unchanged(&self) {
        use std::os::unix::fs::MetadataExt;
        match std::fs::metadata(&self.path) {
            Ok(m) if m.dev() == self.dev && m.ino() == self.ino => {
                let _ = std::fs::remove_file(&self.path);
            }
            _ => {}
        }
    }
}

/// Spawn the relay tasks on `runtime` and return the bound socket's
/// fingerprint so the caller can clean it up on VM exit.
///
/// `sock_path` — where clients connect (e.g. a sandbox's or worker VM's
/// `shell.sock`).
/// `vm_stream` — the host end of the socketpair whose guest half is
/// wired into libkrun as the `iii.exec` virtio-console port.
///
/// On success the relay is running; the returned fingerprint is
/// purely informational/cleanup. On failure (bind failed, couldn't
/// set the VM stream nonblocking) returns `Err` with a human string.
pub fn spawn(
    runtime: &tokio::runtime::Handle,
    sock_path: PathBuf,
    vm_stream: StdUnixStream,
) -> Result<ShellSocketFingerprint, String> {
    // `UnixListener::from_std` / `UnixStream::from_std` register the fd
    // with the current tokio reactor, which panics with "there is no
    // reactor running" if called outside a runtime context. Since
    // `__vm-boot` dispatches on a dedicated `std::thread` (see
    // src/main.rs — workaround for msb_krun's nested
    // tokio::block_on in virtio-blk Drop), we're on a plain OS thread
    // holding only a `Handle`. Enter the runtime for the duration of
    // the setup so the reactor is reachable; the spawned tasks already
    // run inside the runtime and don't need the guard.
    let _runtime_guard = runtime.enter();

    // Narrow umask so the bound socket inode is born with 0o600 mode, so
    // no other local user can connect in the window before
    // `set_permissions`. Best-effort; `set_permissions` below corrects
    // any platform that ignores umask on socket(2).
    prepare_sock_dir(&sock_path)?;

    let fingerprint = {
        let prev = unsafe { libc::umask(0o077) };
        let bind_result = std::os::unix::net::UnixListener::bind(&sock_path);
        unsafe {
            libc::umask(prev);
        }
        match bind_result {
            Ok(listener) => {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&sock_path, std::fs::Permissions::from_mode(0o600));
                let fp = fingerprint_of(&sock_path)?;
                listener
                    .set_nonblocking(true)
                    .map_err(|e| format!("shell_relay: set_nonblocking: {e}"))?;
                let tokio_listener = UnixListener::from_std(listener)
                    .map_err(|e| format!("shell_relay: UnixListener::from_std: {e}"))?;

                // Prepare the VM stream for tokio.
                vm_stream
                    .set_nonblocking(true)
                    .map_err(|e| format!("shell_relay: vm_stream nonblocking: {e}"))?;
                let vm_tokio = tokio::net::UnixStream::from_std(vm_stream)
                    .map_err(|e| format!("shell_relay: UnixStream::from_std: {e}"))?;
                let (vm_read, vm_write) = vm_tokio.into_split();

                // Shared channel feeding the VM writer task. All
                // client → VM frames funnel through here so ordering
                // stays consistent on the wire. Bounded to apply
                // backpressure to any one client that outpaces the
                // virtio-console write speed — see [`VM_WRITE_CAPACITY`].
                let (vm_write_tx, vm_write_rx) = mpsc::channel::<QueuedFrame>(VM_WRITE_CAPACITY);
                let routes: Routes = Arc::new(Mutex::new(HashMap::new()));
                let clients = ClientTable::default();

                // Buffered adapters on the VM halves coalesce the
                // 2 syscalls per frame (4-byte header + body) for
                // small-frame TTY workloads; BufWriter batches the
                // per-disconnect SIGKILL fan-out.
                let vm_read_buffered = tokio::io::BufReader::new(vm_read);
                let vm_write_buffered = tokio::io::BufWriter::new(vm_write);
                runtime.spawn(vm_writer_task(vm_write_buffered, vm_write_rx));
                runtime.spawn(vm_reader_task(
                    vm_read_buffered,
                    routes.clone(),
                    clients.clone(),
                ));
                runtime.spawn(listener_task(tokio_listener, vm_write_tx, routes, clients));
                fp
            }
            Err(e) => {
                return Err(format!(
                    "shell_relay: bind({}) failed: {e}",
                    sock_path.display()
                ));
            }
        }
    };

    Ok(fingerprint)
}

fn prepare_sock_dir(sock_path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(parent) = sock_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("shell_relay: create_dir_all({}): {e}", parent.display()))?;
        let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
    }
    // Remove any stale socket inode so bind() doesn't fail with EADDRINUSE.
    let _ = std::fs::remove_file(sock_path);
    Ok(())
}

fn fingerprint_of(sock_path: &Path) -> Result<ShellSocketFingerprint, String> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(sock_path)
        .map_err(|e| format!("shell_relay: stat({}): {e}", sock_path.display()))?;
    Ok(ShellSocketFingerprint {
        path: sock_path.to_string_lossy().into_owned(),
        dev: meta.dev(),
        ino: meta.ino(),
    })
}

/// Owns the VM write half. Flushes after every frame so
/// latency-sensitive writes (single stdin keystrokes) don't sit in
/// the BufWriter; the buffer exists only to coalesce bursts like the
/// per-disconnect SIGKILL fan-out.
async fn vm_writer_task(
    mut vm_write: tokio::io::BufWriter<tokio::net::unix::OwnedWriteHalf>,
    mut rx: mpsc::Receiver<QueuedFrame>,
) {
    while let Some(frame) = rx.recv().await {
        if let Err(e) = vm_write.write_all(&frame.bytes).await {
            tracing::warn!("shell_relay: vm write failed: {e}");
            break;
        }
        if let Err(e) = vm_write.flush().await {
            tracing::warn!("shell_relay: vm flush failed: {e}");
            break;
        }
    }
}

/// Reads framed bytes from the VM, extracts the `corr_id` + flags
/// header without parsing the JSON payload, and forwards to the
/// owning client's mpsc. Terminal frames remove the route entry
/// after forwarding so the next request with the same id (if any)
/// gets a fresh session.
async fn vm_reader_task(
    mut vm_read: tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>,
    routes: Routes,
    clients: ClientTable,
) {
    loop {
        let frame = match read_frame(&mut vm_read).await {
            Ok(Some(f)) => f,
            Ok(None) => {
                tracing::debug!("shell_relay: vm read EOF");
                break;
            }
            Err(e) => {
                tracing::warn!("shell_relay: vm read error: {e}");
                break;
            }
        };
        // Header: [corr_id: u32 BE][flags: u8]. Already validated by
        // read_frame, so safe to index.
        let corr_id = u32::from_be_bytes([frame[4], frame[5], frame[6], frame[7]]);
        let flags = frame[8];
        let terminal = (flags & FLAG_TERMINAL) != 0;

        let route = {
            let map = routes.lock().expect("routes mutex poisoned");
            map.get(&corr_id)
                .filter(|e| !e.overrun)
                .map(|e| (e.sender.clone(), e.kick.clone()))
        };
        if let Some((tx, kick)) = route {
            // Never waits for a client: that would stall every session.
            // Closed means the client left; its cleanup prunes the route.
            if let Err(mpsc::error::TrySendError::Full(_)) = tx.try_send(frame) {
                if let Some(entry) = routes
                    .lock()
                    .expect("routes mutex poisoned")
                    .get_mut(&corr_id)
                {
                    entry.overrun = true;
                }
                tracing::warn!(
                    "shell_relay: the guest overran corr_id {corr_id}'s window; \
                     ending that session"
                );
                kick.notify_one();
            }
        } else {
            tracing::debug!("shell_relay: dropped frame for unknown corr_id {corr_id}");
        }

        if terminal {
            // Remove from routes AND from the owning client's
            // owned_ids set — without the second step, a client's cap
            // counter would only ever grow, starving legitimate
            // long-running clients after [`MAX_OWNED_IDS_PER_CLIENT`]
            // completed sessions.
            let mut map = routes.lock().expect("routes mutex poisoned");
            match map.remove(&corr_id) {
                Some(entry) => {
                    drop(map);
                    entry
                        .owner_ids
                        .lock()
                        .expect("owner_ids mutex poisoned")
                        .remove(&corr_id);
                }
                // The guest ended a session whose client had left.
                None => {
                    clients
                        .lock()
                        .expect("clients mutex poisoned")
                        .lingering
                        .remove(&corr_id);
                }
            }
        }
    }
}

async fn listener_task(
    listener: UnixListener,
    vm_write_tx: mpsc::Sender<QueuedFrame>,
    routes: Routes,
    clients: ClientTable,
) {
    let our_uid = unsafe { libc::geteuid() };
    loop {
        let (stream, _addr) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("shell_relay: accept failed: {e}");
                // Brief pause so a stuck listener doesn't spin the runtime.
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
        };

        if !peer_uid_matches_tokio(&stream, our_uid) {
            tracing::warn!("shell_relay: rejected client with mismatched uid");
            continue;
        }

        let (vm_write_tx, routes, clients) = (vm_write_tx.clone(), routes.clone(), clients.clone());
        tokio::spawn(async move {
            let Some(id_offset) = wait_for_range(&clients).await else {
                tracing::warn!("shell_relay: MAX_CLIENTS reached, refusing new client");
                // Dropping the stream closes it.
                return;
            };
            client_session(stream, id_offset, vm_write_tx, routes, clients).await;
        });
    }
}

async fn wait_for_range(clients: &ClientTable) -> Option<u32> {
    let deadline = Instant::now() + SLOT_WAIT;
    loop {
        if let Some(id_offset) = claim_range(clients) {
            return Some(id_offset);
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// The next range after the last one handed out that no live client
/// holds and no lingering session uses, or `None` at [`MAX_CLIENTS`].
fn claim_range(clients: &ClientTable) -> Option<u32> {
    let mut clients = clients.lock().expect("clients mutex poisoned");
    if clients.live.len() >= MAX_CLIENTS as usize {
        return None;
    }
    for _ in 0..=u32::MAX / ID_RANGE_STEP {
        let id_offset = clients.next_range;
        clients.next_range = id_offset.wrapping_add(ID_RANGE_STEP);
        let in_use = clients.live.contains(&id_offset)
            || clients
                .lingering
                .iter()
                .any(|id| id.wrapping_sub(id_offset) < ID_RANGE_STEP);
        if !in_use {
            clients.live.insert(id_offset);
            return Some(id_offset);
        }
    }
    None
}

/// Ends a connection: its range is free unless `lingering` sessions in it
/// are still running in the guest.
fn release_range(clients: &ClientTable, id_offset: u32, lingering: Vec<u32>) {
    let mut clients = clients.lock().expect("clients mutex poisoned");
    clients.live.remove(&id_offset);
    clients.lingering.extend(lingering);
}

/// Per-client task. Sends handshake, then concurrently reads client →
/// VM and writes VM → client. Exits on either half closing; on exit
/// releases the client's slot and prunes any routes it still owns.
async fn client_session(
    stream: tokio::net::UnixStream,
    id_offset: u32,
    vm_write_tx: mpsc::Sender<QueuedFrame>,
    routes: Routes,
    clients: ClientTable,
) {
    let id_ceil = id_offset.saturating_add(ID_RANGE_STEP);
    let (mut client_read, mut client_write) = stream.into_split();

    // Handshake: 4-byte id_offset (BE) so the client knows its range.
    if let Err(e) = client_write.write_all(&id_offset.to_be_bytes()).await {
        tracing::debug!("shell_relay: handshake write failed: {e}");
        // Handshake failed before any work — no owned sessions.
        release_range(&clients, id_offset, Vec::new());
        return;
    }

    // Per-client response channel. vm_reader_task pushes here (see
    // [`CLIENT_WRITE_CAPACITY`]); we forward everything to the socket below.
    let (client_tx, mut client_rx) = mpsc::channel::<FrameBytes>(CLIENT_WRITE_CAPACITY);

    // Writer: drain mpsc into client socket, and give each session's
    // guest side credit for the frames its client took.
    let vm_write_tx_for_writer = vm_write_tx.clone();
    let writer_fut = async move {
        let mut delivered: HashMap<u32, u32> = HashMap::new();
        while let Some(frame) = client_rx.recv().await {
            match tokio::time::timeout(CLIENT_STALL, client_write.write_all(&frame)).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => break,
                Err(_) => {
                    tracing::warn!(
                        "shell_relay: a client took nothing for {CLIENT_STALL:?}; ending it"
                    );
                    break;
                }
            }
            let corr_id = u32::from_be_bytes([frame[4], frame[5], frame[6], frame[7]]);
            if frame[8] & FLAG_TERMINAL != 0 {
                delivered.remove(&corr_id);
                continue;
            }
            let count = delivered.entry(corr_id).or_default();
            *count += 1;
            if *count < CREDIT_BATCH {
                continue;
            }
            let frames = std::mem::take(count);
            let Ok(bytes) = encode_frame(corr_id, 0, &ShellMessage::Credit { frames }) else {
                continue;
            };
            let credit = QueuedFrame {
                bytes,
                _guard: None,
            };
            if vm_write_tx_for_writer.send(credit).await.is_err() {
                break;
            }
        }
    };

    // Reader: pull frames from client, register route on first sight
    // of a new corr_id, forward to vm_writer.
    //
    // `owned_ids` is a HashSet (not Vec) so vm_reader_task's terminal
    // handler can decrement it in O(1) on session completion — the
    // set shrinks as sessions finish, which lets a legitimate client
    // run any number of sequential commands without hitting the cap.
    let owned_ids: Arc<Mutex<HashSet<u32>>> = Arc::new(Mutex::new(HashSet::new()));
    let owned_ids_for_reader = owned_ids.clone();
    let routes_for_reader = routes.clone();
    let kick = Arc::new(tokio::sync::Notify::new());
    let kick_for_reader = kick.clone();
    // One-shot log flag: cap-exceeded drops must not spam tracing
    // (an attacker could otherwise flood warn lines to DoS the log
    // sink). Logs once the first time the client hits the cap per
    // session; silent thereafter.
    let cap_logged = Arc::new(AtomicBool::new(false));
    let cap_logged_for_reader = cap_logged.clone();
    let bytes_cap_logged = Arc::new(AtomicBool::new(false));
    let bytes_cap_logged_for_reader = bytes_cap_logged.clone();
    // Per-client bytes-in-flight. Forwarded frames carry a
    // ClientBytesGuard that decrements this on consume; see
    // PER_CLIENT_VM_BYTES_CAP.
    let client_bytes: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
    let client_bytes_for_reader = client_bytes.clone();
    // Keep a clone for the post-disconnect SIGKILL path below; the
    // reader owns its own clone so the moves don't conflict.
    let vm_write_tx_for_reader = vm_write_tx.clone();
    let reader_fut = async move {
        loop {
            let frame = match read_frame(&mut client_read).await {
                Ok(Some(f)) => f,
                _ => break,
            };
            let corr_id = u32::from_be_bytes([frame[4], frame[5], frame[6], frame[7]]);
            if corr_id <= id_offset || corr_id >= id_ceil {
                tracing::warn!(
                    "shell_relay: client sent corr_id {corr_id} outside its range \
                     ({}, {})",
                    id_offset,
                    id_ceil
                );
                // Drop the frame. Don't close — a single bad id
                // shouldn't kill the whole session.
                continue;
            }
            // Register route if new. Track for cleanup.
            //
            // Hard cap per-client route registrations at
            // [`MAX_OWNED_IDS_PER_CLIENT`] to deny a same-uid peer a
            // cheap memory-exhaustion vector (flooding distinct
            // corr_ids). When the cap is hit, drop the frame on the
            // floor — don't forward to the VM either, because a
            // guest-side session for an unrouted corr_id would be
            // effectively unreachable (responses have nowhere to go).
            //
            // Lock order: owned_ids FIRST, then routes. Keeps
            // vm_reader_task's hot-path `routes.lock()` critical
            // section tight — it never has to wait on cap accounting.
            {
                use std::collections::hash_map::Entry;
                let mut owned_guard = owned_ids_for_reader
                    .lock()
                    .expect("owned_ids mutex poisoned");
                let mut map = routes_for_reader.lock().expect("routes mutex poisoned");
                if let Entry::Vacant(slot) = map.entry(corr_id) {
                    if owned_guard.len() >= MAX_OWNED_IDS_PER_CLIENT {
                        drop(map);
                        drop(owned_guard);
                        if !cap_logged_for_reader.swap(true, Ordering::Relaxed) {
                            tracing::warn!(
                                "shell_relay: client reached owned-ids cap \
                                 ({MAX_OWNED_IDS_PER_CLIENT}); dropping frames \
                                 for new corr_ids (logging once per session)"
                            );
                        }
                        continue;
                    }
                    slot.insert(RouteEntry {
                        sender: client_tx.clone(),
                        owner_ids: owned_ids_for_reader.clone(),
                        kick: kick_for_reader.clone(),
                        overrun: false,
                    });
                    owned_guard.insert(corr_id);
                }
            }
            // Reserve bytes atomically; unwind and drop if over cap.
            let frame_len = frame.len() as u64;
            let before = client_bytes_for_reader.fetch_add(frame_len, Ordering::AcqRel);
            if before.saturating_add(frame_len) > PER_CLIENT_VM_BYTES_CAP {
                client_bytes_for_reader.fetch_sub(frame_len, Ordering::Release);
                if !bytes_cap_logged_for_reader.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        "shell_relay: client reached bytes-in-flight cap \
                         ({PER_CLIENT_VM_BYTES_CAP} bytes); dropping frame \
                         corr_id={corr_id} (logging once per session)"
                    );
                }
                continue;
            }
            let queued = QueuedFrame {
                bytes: frame,
                _guard: Some(ClientBytesGuard {
                    counter: client_bytes_for_reader.clone(),
                    size: frame_len,
                }),
            };
            // Bounded `.send().await`: applies backpressure to this
            // one reader when the VM queue is full, which is exactly
            // the behavior we want. A flooding client stalls on its
            // own read loop rather than ballooning relay memory.
            if vm_write_tx_for_reader.send(queued).await.is_err() {
                break;
            }
        }
    };

    // Either side closing ends the session. Drop the writer's mpsc
    // sender by completing reader_fut (which owns the clone via
    // `client_tx` inside the routes map). We cope by select!-ing on
    // both halves and breaking when either completes.
    tokio::select! {
        _ = reader_fut => {}
        _ = writer_fut => {}
        _ = kick.notified() => {}
    }

    // On client disconnect, any sessions this client spawned in the
    // guest are still running — the guest dispatcher has no way to
    // know the host side went away. Deliver SIGKILL to every owned
    // session so the guest kills its child or stops its read.
    // Snapshot the still-live ids, then send SIGKILL for each. With
    // vm_reader_task decrementing owned_ids on every terminal frame,
    // this set only contains sessions that were still running when
    // the client went away — no wasted SIGKILLs for already-completed
    // sessions.
    let ids: Vec<u32> = owned_ids
        .lock()
        .expect("owned_ids mutex poisoned")
        .iter()
        .copied()
        .collect();
    for &id in &ids {
        let frame = match encode_frame(id, 0, &ShellMessage::Signal { signal: 9 }) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("shell_relay: failed to encode SIGKILL frame for corr_id={id}: {e}");
                continue;
            }
        };
        // Internal-source frame: no guard (client counter is being
        // torn down). try_send first so parallel disconnects don't
        // serialize; await on Full so SIGKILL can't be dropped.
        let queued = QueuedFrame {
            bytes: frame,
            _guard: None,
        };
        match vm_write_tx.try_send(queued) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(queued)) => {
                let _ = vm_write_tx.send(queued).await;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => break,
        }
    }
    // The slot is free at once. A session whose route is still here has
    // not ended: its late frames find no route, and its id keeps the
    // range out of use until the guest ends it. The routes lock covers
    // the hand-over, so vm_reader_task's terminal frame cannot slip
    // between the two.
    let mut owned = owned_ids.lock().expect("owned_ids mutex poisoned");
    let mut map = routes.lock().expect("routes mutex poisoned");
    let lingering = owned
        .drain()
        .filter(|id| map.remove(id).is_some())
        .collect();
    release_range(&clients, id_offset, lingering);
}

/// Read one full frame (length prefix + body). `Ok(None)` on EOF at
/// a frame boundary. Returned buffer includes the 4-byte length
/// prefix so callers forward verbatim.
async fn read_frame<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<FrameBytes>> {
    let mut len_buf = [0u8; 4];
    if read_exact_or_eof(reader, &mut len_buf).await?.is_none() {
        return Ok(None);
    }
    let frame_len = u32::from_be_bytes(len_buf) as usize;
    if !(FRAME_HEADER_SIZE..=MAX_FRAME_SIZE).contains(&frame_len) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("frame length {frame_len} out of range"),
        ));
    }
    // Incremental read with a capped initial allocation (no uninit memory,
    // no full-size zero-fill): memory grows only as payload arrives, so a
    // peer sending a max-size length prefix and stalling costs at most
    // FRAME_READ_INITIAL_CAP, not MAX_FRAME_SIZE.
    let mut buf: FrameBytes =
        Vec::with_capacity((4 + frame_len).min(iii_shell_proto::FRAME_READ_INITIAL_CAP));
    buf.extend_from_slice(&len_buf);
    let read = (&mut *reader)
        .take(frame_len as u64)
        .read_to_end(&mut buf)
        .await?;
    if read != frame_len {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "partial frame body",
        ));
    }
    Ok(Some(buf))
}

/// Try to fill `buf`, distinguishing EOF-at-boundary (`Ok(None)`)
/// from partial reads (`Err(UnexpectedEof)`).
async fn read_exact_or_eof<R: AsyncReadExt + Unpin>(
    reader: &mut R,
    buf: &mut [u8],
) -> std::io::Result<Option<()>> {
    let mut read = 0;
    while read < buf.len() {
        match reader.read(&mut buf[read..]).await? {
            0 => {
                if read == 0 {
                    return Ok(None);
                }
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "partial read",
                ));
            }
            n => read += n,
        }
    }
    Ok(Some(()))
}

/// `SO_PEERCRED` (Linux) / `getpeereid` (macOS/BSD): does the
/// connecting peer share our euid? Tokio's `UnixStream` exposes the
/// raw fd, so we reuse the libc incantation directly.
fn peer_uid_matches_tokio(stream: &tokio::net::UnixStream, expected: u32) -> bool {
    use std::os::unix::io::AsRawFd;
    let fd = stream.as_raw_fd();

    #[cfg(target_os = "linux")]
    {
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len: libc::socklen_t = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut cred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        return rc == 0 && cred.uid == expected;
    }

    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd",
    ))]
    {
        let mut uid: libc::uid_t = 0;
        let mut gid: libc::gid_t = 0;
        let rc = unsafe { libc::getpeereid(fd, &mut uid, &mut gid) };
        return rc == 0 && uid == expected;
    }

    #[allow(unreachable_code)]
    {
        let _ = (fd, expected);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iii_supervisor::shell_protocol::{
        ShellMessage, encode_frame, flags::FLAG_TERMINAL, read_frame_blocking,
    };
    use std::os::unix::net::UnixStream as StdUS;
    use std::time::Duration;
    use tempfile::TempDir;

    /// Boot the relay with a socketpair standing in for the VM end.
    /// Returns (sock_path, vm_host_end_simulator, tmpdir-for-lifetime).
    async fn boot() -> (PathBuf, StdUS, TempDir) {
        let tmp = TempDir::new().unwrap();
        let sock_path = tmp.path().join("shell.sock");
        let (vm_host, vm_guest_sim) = StdUS::pair().unwrap();
        // The relay owns vm_host (wrote into libkrun in production).
        // The test keeps vm_guest_sim and pretends to be the guest
        // dispatcher, reading requests and writing responses.
        let runtime = tokio::runtime::Handle::current();
        spawn(&runtime, sock_path.clone(), vm_host).expect("relay spawn");
        // Poll for the bound inode. Avoids a probe-connect that
        // would take the first id range the tests expect.
        let deadline = Duration::from_secs(2);
        let start = std::time::Instant::now();
        loop {
            if std::fs::metadata(&sock_path).is_ok() {
                break;
            }
            if start.elapsed() >= deadline {
                panic!("listener did not bind within {deadline:?}");
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        (sock_path, vm_guest_sim, tmp)
    }

    #[tokio::test]
    async fn handshake_sends_id_offset() {
        let (sock, _vm_guest_sim, _tmp) = boot().await;
        let mut client = tokio::net::UnixStream::connect(&sock).await.unwrap();
        let mut buf = [0u8; 4];
        client.read_exact(&mut buf).await.unwrap();
        let id_offset = u32::from_be_bytes(buf);
        // First client gets the first range → id_offset = 0.
        assert_eq!(id_offset, 0);
    }

    #[tokio::test]
    async fn second_client_gets_different_offset() {
        let (sock, _vm_guest_sim, _tmp) = boot().await;
        let mut c1 = tokio::net::UnixStream::connect(&sock).await.unwrap();
        let mut c2 = tokio::net::UnixStream::connect(&sock).await.unwrap();
        let mut b1 = [0u8; 4];
        let mut b2 = [0u8; 4];
        c1.read_exact(&mut b1).await.unwrap();
        c2.read_exact(&mut b2).await.unwrap();
        let off1 = u32::from_be_bytes(b1);
        let off2 = u32::from_be_bytes(b2);
        assert_ne!(off1, off2);
        // Difference is one full range step.
        assert_eq!(off2 - off1, ID_RANGE_STEP);
    }

    #[tokio::test]
    async fn end_to_end_routes_round_trip_through_vm_simulator() {
        let (sock, vm_guest_sim, _tmp) = boot().await;
        // Fire up a tiny "guest simulator" in a thread: reads frames
        // from the VM side of the socketpair, echoes an Exited reply
        // with the same corr_id + terminal flag.
        let guest_clone = vm_guest_sim.try_clone().unwrap();
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(vm_guest_sim);
            let mut writer = guest_clone;
            while let Ok(Some((corr_id, _, _))) = read_frame_blocking(&mut reader) {
                let reply = encode_frame(corr_id, FLAG_TERMINAL, &ShellMessage::Exited { code: 0 })
                    .unwrap();
                use std::io::Write;
                writer.write_all(&reply).unwrap();
            }
        });

        let mut client = tokio::net::UnixStream::connect(&sock).await.unwrap();
        let mut handshake = [0u8; 4];
        client.read_exact(&mut handshake).await.unwrap();
        let id_offset = u32::from_be_bytes(handshake);
        let corr_id = id_offset + 1;

        // Send a Request. Relay doesn't care about the variant — it
        // just routes by header — so we can use a minimal valid one.
        let req = ShellMessage::Request {
            cmd: "/bin/true".into(),
            args: vec![],
            env: vec![],
            cwd: None,
            tty: false,
            rows: 24,
            cols: 80,
        };
        let frame = encode_frame(corr_id, 0, &req).unwrap();
        client.write_all(&frame).await.unwrap();

        // Read the reply the simulator sent. Use a deadline so a
        // broken routing table fails loudly instead of hanging.
        let reply = tokio::time::timeout(Duration::from_secs(2), read_frame(&mut client))
            .await
            .expect("timeout")
            .expect("read ok")
            .expect("frame");
        // Assert header matches.
        let reply_corr = u32::from_be_bytes([reply[4], reply[5], reply[6], reply[7]]);
        let reply_flags = reply[8];
        assert_eq!(reply_corr, corr_id);
        assert!(reply_flags & FLAG_TERMINAL != 0);
    }

    #[tokio::test]
    async fn client_outside_its_range_is_dropped_but_session_continues() {
        let (sock, vm_guest_sim, _tmp) = boot().await;
        // Guest echoes whatever it sees.
        let guest_clone = vm_guest_sim.try_clone().unwrap();
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(vm_guest_sim);
            let mut writer = guest_clone;
            while let Ok(Some((corr_id, _, _))) = read_frame_blocking(&mut reader) {
                let reply = encode_frame(corr_id, FLAG_TERMINAL, &ShellMessage::Exited { code: 0 })
                    .unwrap();
                use std::io::Write;
                writer.write_all(&reply).unwrap();
            }
        });

        let mut client = tokio::net::UnixStream::connect(&sock).await.unwrap();
        let mut handshake = [0u8; 4];
        client.read_exact(&mut handshake).await.unwrap();
        let id_offset = u32::from_be_bytes(handshake);

        // Send a frame with an out-of-range corr_id. Relay should
        // drop it without closing the connection.
        let bogus_id = id_offset.saturating_add(ID_RANGE_STEP + 1);
        let bogus = encode_frame(bogus_id, 0, &ShellMessage::Signal { signal: 9 }).unwrap();
        client.write_all(&bogus).await.unwrap();

        // Follow with a valid frame — the relay should still serve it.
        let valid_id = id_offset + 1;
        let valid = encode_frame(
            valid_id,
            0,
            &ShellMessage::Request {
                cmd: "/bin/true".into(),
                args: vec![],
                env: vec![],
                cwd: None,
                tty: false,
                rows: 24,
                cols: 80,
            },
        )
        .unwrap();
        client.write_all(&valid).await.unwrap();

        let reply = tokio::time::timeout(Duration::from_secs(2), read_frame(&mut client))
            .await
            .expect("timeout")
            .expect("read ok")
            .expect("frame");
        let reply_corr = u32::from_be_bytes([reply[4], reply[5], reply[6], reply[7]]);
        assert_eq!(reply_corr, valid_id, "valid frame must still route");
    }

    /// REGRESSION: a tight burst of clean (no owned-id) disconnects
    /// must NOT exhaust the slot pool.
    ///
    /// Prior bug: every disconnect put the slot into a 250 ms
    /// `Cooldown` window unconditionally. With `MAX_CLIENTS=16`, a
    /// burst of 17+ fast sequential connections (tens of ms each)
    /// finished before slot 0 cooled, so the 17th claim_slot found
    /// every slot still cooling and the listener refused with
    /// `early eof on handshake` → host saw S216
    /// (`relay rejected connection (uid mismatch?)`). Real-world
    /// repro: `fs-example.mjs` running ~14 happy-path triggers then
    /// a few adversarial-section setup triggers in <250 ms.
    ///
    /// Fix: only enter Cooldown when SIGKILL fan-out actually
    /// happened (owned_ids non-empty at session end). Clean
    /// disconnects — every FS trigger that received its terminal
    /// frame, every cleanly-exited exec — drop straight to Free,
    /// so back-to-back trigger bursts don't burn slot capacity.
    ///
    /// This test fires 32 sequential connect/handshake/disconnect
    /// cycles — twice MAX_CLIENTS — and asserts every one received
    /// its 4-byte handshake. Under the old behavior the 17th would
    /// time out on read; under the fix all 32 succeed.
    #[tokio::test]
    async fn burst_of_clean_disconnects_does_not_exhaust_slots() {
        let (sock, _vm_guest_sim, _tmp) = boot().await;

        // 2 × MAX_CLIENTS sequential connect → read handshake → drop.
        // Each cycle's UnixStream goes out of scope at end-of-iteration
        // so the relay sees a clean EOF on read_frame, exits the
        // session, and frees its slot.
        for i in 0..(MAX_CLIENTS as usize * 2) {
            let mut s = tokio::net::UnixStream::connect(&sock)
                .await
                .unwrap_or_else(|e| panic!("connect on iteration {i} failed: {e}"));
            let mut buf = [0u8; 4];
            tokio::time::timeout(Duration::from_millis(500), s.read_exact(&mut buf))
                .await
                .unwrap_or_else(|_| panic!("iteration {i}: handshake read timed out"))
                .unwrap_or_else(|e| panic!("iteration {i}: handshake failed: {e}"));
            // Drop here — the UnixStream's Drop closes both halves,
            // the relay's reader_fut sees EOF, client_session exits
            // with empty owned_ids, and its slot is free.
            drop(s);
            // Tiny yield so the relay's per-session cleanup actually
            // schedules before we open the next connection. Without
            // this the test stays correct but exercises a different
            // race (claim_range vs release_range ordering on the same
            // tokio task) — keeping it deterministic helps.
            tokio::task::yield_now().await;
        }
    }

    /// A timed-out exec drops its connection with the session still live.
    /// Its slot is free at once: 2 × MAX_CLIENTS such sessions in a row all
    /// get a slot, where a fixed cooldown refused the 17th.
    #[tokio::test]
    async fn burst_of_killed_sessions_does_not_exhaust_slots() {
        let (sock, vm_guest_sim, _tmp) = boot().await;
        // Guest simulator: ends a session when it is SIGKILLed.
        let mut writer = vm_guest_sim.try_clone().unwrap();
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(vm_guest_sim);
            while let Ok(Some((corr_id, _, msg))) = read_frame_blocking(&mut reader) {
                if matches!(msg, ShellMessage::Signal { signal: 9 }) {
                    let exited = ShellMessage::Exited { code: 137 };
                    let reply = encode_frame(corr_id, FLAG_TERMINAL, &exited).unwrap();
                    use std::io::Write;
                    writer.write_all(&reply).unwrap();
                }
            }
        });
        let request = ShellMessage::Request {
            cmd: "/bin/sleep".into(),
            args: vec!["30".into()],
            env: vec![],
            cwd: None,
            tty: false,
            rows: 24,
            cols: 80,
        };
        for i in 0..(MAX_CLIENTS as usize * 2) {
            let mut s = tokio::net::UnixStream::connect(&sock).await.unwrap();
            let mut hs = [0u8; 4];
            tokio::time::timeout(Duration::from_millis(500), s.read_exact(&mut hs))
                .await
                .unwrap_or_else(|_| panic!("session {i}: handshake read timed out"))
                .unwrap_or_else(|e| panic!("session {i}: refused: {e}"));
            let corr_id = u32::from_be_bytes(hs) + 1;
            s.write_all(&encode_frame(corr_id, 0, &request).unwrap())
                .await
                .unwrap();
            // Dropped mid-session, as on an exec timeout.
            drop(s);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// A guest simulator's half of flow control: send `frames` output
    /// frames for `corr_id` at most [`SEND_WINDOW`] ahead of the relay's
    /// `Credit` frames, then the session's end.
    fn stream_with_credit(
        reader: &mut std::io::BufReader<StdUS>,
        writer: &mut StdUS,
        corr_id: u32,
        frames: usize,
        mut credit: u32,
    ) {
        use std::io::Write;
        let chunk = ShellMessage::Stdout {
            data_b64: "A".repeat(1024),
        };
        let chunk = encode_frame(corr_id, 0, &chunk).unwrap();
        let mut sent = 0;
        while sent < frames {
            while credit > 0 && sent < frames {
                writer.write_all(&chunk).unwrap();
                credit -= 1;
                sent += 1;
            }
            match read_frame_blocking(reader) {
                Ok(Some((id, _, ShellMessage::Credit { frames }))) if id == corr_id => {
                    credit += frames
                }
                Ok(Some(_)) => {}
                _ => return,
            }
        }
        let end = encode_frame(corr_id, FLAG_TERMINAL, &ShellMessage::Exited { code: 0 });
        writer.write_all(&end.unwrap()).unwrap();
    }

    fn cat_request() -> ShellMessage {
        ShellMessage::Request {
            cmd: "/bin/cat".into(),
            args: vec![],
            env: vec![],
            cwd: None,
            tty: false,
            rows: 24,
            cols: 80,
        }
    }

    /// Connect, read the handshake and send `request` as the session's
    /// first frame. Returns the stream and the session's corr_id.
    async fn open_session(sock: &Path, request: &ShellMessage) -> (tokio::net::UnixStream, u32) {
        let mut client = tokio::net::UnixStream::connect(sock).await.unwrap();
        let mut hs = [0u8; 4];
        client.read_exact(&mut hs).await.unwrap();
        let corr_id = u32::from_be_bytes(hs) + 1;
        client
            .write_all(&encode_frame(corr_id, 0, request).unwrap())
            .await
            .unwrap();
        (client, corr_id)
    }

    /// A client that stops reading (a large `fs::read` whose consumer is
    /// slow) holds back only its own session: the guest waits for its
    /// credit, the VM's other session gets its frames meanwhile, and the
    /// slow stream still arrives whole, far past the relay's own buffer.
    #[tokio::test]
    async fn a_slow_reader_holds_back_only_its_own_session() {
        const FRAMES: usize = 3 * CLIENT_WRITE_CAPACITY;
        let (sock, vm_guest_sim, _tmp) = boot().await;
        // Guest simulator: a window for the slow session, the other
        // session's end, then the rest of the slow one as credit allows.
        let mut writer = vm_guest_sim.try_clone().unwrap();
        std::thread::spawn(move || {
            use std::io::Write;
            let mut reader = std::io::BufReader::new(vm_guest_sim);
            let mut ids = Vec::new();
            while ids.len() < 2 {
                match read_frame_blocking(&mut reader) {
                    Ok(Some((corr_id, _, _))) => ids.push(corr_id),
                    _ => return,
                }
            }
            let chunk = ShellMessage::Stdout {
                data_b64: "A".repeat(1024),
            };
            let chunk = encode_frame(ids[0], 0, &chunk).unwrap();
            for _ in 0..SEND_WINDOW {
                writer.write_all(&chunk).unwrap();
            }
            let end = encode_frame(ids[1], FLAG_TERMINAL, &ShellMessage::Exited { code: 0 });
            writer.write_all(&end.unwrap()).unwrap();
            let rest = FRAMES - SEND_WINDOW as usize;
            stream_with_credit(&mut reader, &mut writer, ids[0], rest, 0);
        });
        let (mut slow, _) = open_session(&sock, &cat_request()).await;
        // So the guest sees the slow session's request first.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let (mut other, _) = open_session(&sock, &cat_request()).await;

        let end = tokio::time::timeout(Duration::from_secs(2), read_frame(&mut other))
            .await
            .expect("the other session was held up behind the slow one")
            .unwrap()
            .expect("frame");
        assert!(end[8] & FLAG_TERMINAL != 0);

        tokio::time::sleep(Duration::from_millis(300)).await;
        let mut chunks = 0;
        loop {
            let frame = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut slow))
                .await
                .expect("stream stalled")
                .unwrap()
                .expect("stream ended before its terminal frame");
            if frame[8] & FLAG_TERMINAL != 0 {
                break;
            }
            chunks += 1;
        }
        assert_eq!(chunks, FRAMES);
    }

    /// A guest that ignores a session's window cannot make the relay wait:
    /// the overrun session alone is cut (its client sees the connection
    /// close before a terminal frame), and the other session is served.
    #[tokio::test]
    async fn a_session_whose_guest_overruns_its_window_fails_alone() {
        let (sock, vm_guest_sim, _tmp) = boot().await;
        let mut writer = vm_guest_sim.try_clone().unwrap();
        std::thread::spawn(move || {
            use std::io::Write;
            let mut reader = std::io::BufReader::new(vm_guest_sim);
            let mut ids = Vec::new();
            while ids.len() < 2 {
                match read_frame_blocking(&mut reader) {
                    Ok(Some((corr_id, _, _))) => ids.push(corr_id),
                    _ => return,
                }
            }
            let chunk = ShellMessage::Stdout {
                data_b64: "A".repeat(1024),
            };
            let chunk = encode_frame(ids[0], 0, &chunk).unwrap();
            for _ in 0..2 * CLIENT_WRITE_CAPACITY {
                if writer.write_all(&chunk).is_err() {
                    return;
                }
            }
            let end = encode_frame(ids[1], FLAG_TERMINAL, &ShellMessage::Exited { code: 0 });
            let _ = writer.write_all(&end.unwrap());
        });
        let (mut overrun, _) = open_session(&sock, &cat_request()).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let (mut other, _) = open_session(&sock, &cat_request()).await;

        let end = tokio::time::timeout(Duration::from_secs(3), read_frame(&mut other))
            .await
            .expect("the other session was held up behind the overrun one")
            .unwrap()
            .expect("frame");
        assert!(end[8] & FLAG_TERMINAL != 0);
        let cut = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match read_frame(&mut overrun).await {
                    Ok(Some(frame)) if frame[8] & FLAG_TERMINAL != 0 => return false,
                    Ok(Some(_)) => {}
                    Ok(None) | Err(_) => return true,
                }
            }
        })
        .await
        .expect("the overrun session was never cut");
        assert!(cut, "the overrun session ended as if whole");
    }

    /// A client that takes no frame for CLIENT_STALL is ended and its
    /// session SIGKILLed, so an `fs::read` nobody reads cannot hold a relay
    /// slot and a guest thread until the sandbox is reaped.
    #[tokio::test]
    async fn a_client_that_takes_nothing_is_ended_and_its_session_killed() {
        let (sock, vm_guest_sim, _tmp) = boot().await;
        let mut writer = vm_guest_sim.try_clone().unwrap();
        let (killed_tx, killed_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::Write;
            let mut reader = std::io::BufReader::new(vm_guest_sim);
            let Ok(Some((corr_id, _, _))) = read_frame_blocking(&mut reader) else {
                return;
            };
            // A window of large frames: more than any socket buffer holds.
            let chunk = ShellMessage::Stdout {
                data_b64: "A".repeat(64 * 1024),
            };
            let chunk = encode_frame(corr_id, 0, &chunk).unwrap();
            for _ in 0..SEND_WINDOW {
                writer.write_all(&chunk).unwrap();
            }
            while let Ok(Some((id, _, msg))) = read_frame_blocking(&mut reader) {
                if id == corr_id && msg == (ShellMessage::Signal { signal: 9 }) {
                    let _ = killed_tx.send(());
                    return;
                }
            }
        });
        let (mut idle, _) = open_session(&sock, &cat_request()).await;
        let started = Instant::now();
        tokio::task::spawn_blocking(move || killed_rx.recv_timeout(Duration::from_secs(5)))
            .await
            .unwrap()
            .expect("the stalled session was never SIGKILLed");
        assert!(started.elapsed() >= CLIENT_STALL);
        let closed = tokio::time::timeout(Duration::from_secs(5), async {
            while let Ok(Some(_)) = read_frame(&mut idle).await {}
        })
        .await;
        assert!(
            closed.is_ok(),
            "the relay kept the stalled client connected"
        );
    }

    /// A client over the cap waits briefly for a slot instead of being
    /// refused at once: a slot comes back as soon as its client leaves.
    #[tokio::test]
    async fn a_client_over_the_cap_waits_for_a_slot() {
        let (sock, _vm_guest_sim, _tmp) = boot().await;
        let mut held = Vec::new();
        for _ in 0..MAX_CLIENTS {
            let mut s = tokio::net::UnixStream::connect(&sock).await.unwrap();
            let mut buf = [0u8; 4];
            s.read_exact(&mut buf).await.unwrap();
            held.push(s);
        }
        let mut late = tokio::net::UnixStream::connect(&sock).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        // A clean disconnect frees its slot at once.
        drop(held.pop());

        let mut buf = [0u8; 4];
        tokio::time::timeout(Duration::from_secs(1), late.read_exact(&mut buf))
            .await
            .expect("no handshake")
            .expect("refused instead of waiting for the freed slot");
    }

    #[tokio::test]
    async fn seventeenth_client_is_refused() {
        let (sock, _vm_guest_sim, _tmp) = boot().await;
        // Open 16 clients and hold them.
        let mut held = Vec::new();
        for _ in 0..MAX_CLIENTS {
            let mut s = tokio::net::UnixStream::connect(&sock).await.unwrap();
            let mut buf = [0u8; 4];
            s.read_exact(&mut buf).await.unwrap();
            held.push(s);
        }
        // 17th: connect succeeds at the TCP-stack level but the
        // relay closes us without a handshake. read_exact should
        // hit EOF.
        let mut s = tokio::net::UnixStream::connect(&sock).await.unwrap();
        let mut buf = [0u8; 4];
        let res = tokio::time::timeout(Duration::from_millis(500), s.read_exact(&mut buf)).await;
        match res {
            Ok(Ok(_)) => panic!("17th client got a handshake — expected refusal"),
            Ok(Err(_)) | Err(_) => {
                // Either an io error or timeout — both acceptable;
                // the defining property is "no handshake".
            }
        }
    }

    #[tokio::test]
    async fn removes_stale_socket_before_bind() {
        let tmp = TempDir::new().unwrap();
        let sock_path = tmp.path().join("shell.sock");
        // Plant a stale file at the path. bind() would otherwise
        // fail with EADDRINUSE.
        std::fs::write(&sock_path, b"stale").unwrap();

        let (vm_host, _vm_guest) = StdUS::pair().unwrap();
        let runtime = tokio::runtime::Handle::current();
        let fp = spawn(&runtime, sock_path.clone(), vm_host).expect("spawn");
        assert!(sock_path.exists());
        // Fingerprint matches the live socket.
        let live_meta = std::fs::metadata(&sock_path).unwrap();
        use std::os::unix::fs::MetadataExt;
        assert_eq!(fp.dev, live_meta.dev());
        assert_eq!(fp.ino, live_meta.ino());
    }

    /// REGRESSION for the "per-client owned-ids unbounded growth" DoS
    /// **and** the coupled "completed sessions never decrement the cap"
    /// bug that the first fix attempt introduced.
    ///
    /// Prior behavior (original bug): a same-uid client could flood
    /// distinct `corr_id`s and get every one registered as a route,
    /// pinning unbounded memory (~6 GiB per slot worst case).
    ///
    /// Second-attempt bug: the owned_ids Vec grew on every new id but
    /// was never decremented when vm_reader_task saw FLAG_TERMINAL,
    /// so a legitimate client ran into the cap after running
    /// MAX_OWNED_IDS_PER_CLIENT sequential commands — even though the
    /// relay's memory footprint stayed flat.
    ///
    /// Current behavior: the cap blocks floods, AND terminal frames
    /// shrink owned_ids so well-behaved clients can run unlimited
    /// sequential sessions. The test covers BOTH axes: it floods with
    /// cap+extra distinct ids and asserts the VM saw exactly `cap` of
    /// them (the flood-cap property), then sends extra requests after
    /// the simulator has replied with terminal frames for a chunk of
    /// the initial ids, and asserts each extra id also makes it
    /// through (proves the cap decremented).
    #[tokio::test]
    async fn owned_ids_capped_per_client() {
        use iii_supervisor::shell_protocol::{
            ShellMessage as SM, encode_frame, read_frame_blocking,
        };

        let (sock, vm_guest_sim, _tmp) = boot().await;

        // VM simulator: records every distinct corr_id it sees, and
        // (via a control channel) can be told to reply with a
        // terminal Exited frame for a specific id so we can exercise
        // the vm_reader_task decrement path.
        let seen: Arc<Mutex<std::collections::HashSet<u32>>> =
            Arc::new(Mutex::new(std::collections::HashSet::new()));
        let (reply_tx, reply_rx) = std::sync::mpsc::channel::<u32>();
        let seen_for_thread = seen.clone();
        let guest_clone = vm_guest_sim.try_clone().unwrap();
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(vm_guest_sim);
            let mut writer = guest_clone;
            std::thread::spawn(move || {
                use std::io::Write;
                while let Ok(id) = reply_rx.recv() {
                    let reply = encode_frame(
                        id,
                        iii_supervisor::shell_protocol::flags::FLAG_TERMINAL,
                        &SM::Exited { code: 0 },
                    )
                    .unwrap();
                    if writer.write_all(&reply).is_err() {
                        break;
                    }
                }
            });
            while let Ok(Some((corr_id, _, _))) = read_frame_blocking(&mut reader) {
                seen_for_thread
                    .lock()
                    .expect("seen poisoned")
                    .insert(corr_id);
            }
        });

        let mut client = tokio::net::UnixStream::connect(&sock).await.unwrap();
        let mut hs = [0u8; 4];
        client.read_exact(&mut hs).await.unwrap();
        let off = u32::from_be_bytes(hs);

        // Phase 1 — flood cap+over distinct Requests without
        // terminal replies. Only the first `cap` should reach the VM.
        let over = 32u32;
        for i in 0..(MAX_OWNED_IDS_PER_CLIENT as u32 + over) {
            let req = SM::Request {
                cmd: "/bin/true".into(),
                args: vec![],
                env: vec![],
                cwd: None,
                tty: false,
                rows: 24,
                cols: 80,
            };
            let frame = encode_frame(off + 1 + i, 0, &req).unwrap();
            client.write_all(&frame).await.unwrap();
        }

        // Deadline-poll for the simulator to observe `cap` distinct
        // ids — the relay never forwards more, so a stable read is
        // the done condition.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let distinct = seen.lock().unwrap().len();
            if distinct >= MAX_OWNED_IDS_PER_CLIENT {
                break;
            }
            if std::time::Instant::now() > deadline {
                panic!(
                    "flood phase timeout: expected {MAX_OWNED_IDS_PER_CLIENT} \
                     distinct corr_ids at the VM simulator within 5s, saw \
                     {distinct}"
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        {
            let distinct = seen.lock().unwrap().len();
            assert_eq!(
                distinct, MAX_OWNED_IDS_PER_CLIENT,
                "flood phase: expected relay to forward exactly \
                 {MAX_OWNED_IDS_PER_CLIENT} distinct corr_ids (cap), saw \
                 {distinct}"
            );
        }

        // Phase 2 — terminal Exited frames for the first 64 ids
        // should decrement the cap. The decrement is entirely
        // host-side and has no client-visible signal to poll on
        // (test client doesn't read client_rx), so a bounded sleep
        // is the least-bad option. 500ms is well above observed
        // worst case (microseconds).
        let drain = 64u32;
        for i in 0..drain {
            reply_tx.send(off + 1 + i).unwrap();
        }
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Phase 3 — send `drain` NEW distinct ids past the original
        // cap boundary. They should now fit because the earlier
        // terminals freed up room. If owned_ids weren't decrementing,
        // every one of these would be dropped at the relay and
        // `seen.len()` would stay stuck at MAX_OWNED_IDS_PER_CLIENT.
        let fresh_base = off + 1 + MAX_OWNED_IDS_PER_CLIENT as u32 + over + 1;
        for i in 0..drain {
            let req = SM::Request {
                cmd: "/bin/true".into(),
                args: vec![],
                env: vec![],
                cwd: None,
                tty: false,
                rows: 24,
                cols: 80,
            };
            let frame = encode_frame(fresh_base + i, 0, &req).unwrap();
            client.write_all(&frame).await.unwrap();
        }

        let expected = MAX_OWNED_IDS_PER_CLIENT + drain as usize;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let distinct = seen.lock().unwrap().len();
            if distinct >= expected {
                break;
            }
            if std::time::Instant::now() > deadline {
                panic!(
                    "decrement phase timeout: expected {expected} distinct \
                     ids within 5s, saw {distinct}"
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let distinct = seen.lock().unwrap().len();
        assert_eq!(
            distinct,
            MAX_OWNED_IDS_PER_CLIENT + drain as usize,
            "decrement phase: after draining {drain} sessions with \
             terminal frames, {drain} NEW ids should have gotten \
             through the cap, got {distinct}"
        );
    }

    /// REGRESSION for the slot-reuse leak: a client left with its session
    /// still running in the guest, and the next client was handed the same
    /// `corr_id`, so the old session's late frames, its end included,
    /// reached the new one. The old session is SIGKILLed, and a new client
    /// always gets ids of its own, however long after the old one left.
    #[tokio::test]
    async fn an_abandoned_sessions_late_frames_never_reach_the_next_client() {
        let (sock, vm_guest_sim, _tmp) = boot().await;
        // Guest simulator: keeps the first session running past its
        // SIGKILL, and answers the next request only after a late frame
        // and the end of the first one.
        let mut writer = vm_guest_sim.try_clone().unwrap();
        let (seen_tx, seen_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::Write;
            let mut reader = std::io::BufReader::new(vm_guest_sim);
            let mut first = None;
            while let Ok(Some((corr_id, _, msg))) = read_frame_blocking(&mut reader) {
                let request = matches!(msg, ShellMessage::Request { .. });
                let _ = seen_tx.send((corr_id, msg));
                match (request, first) {
                    (false, _) => {}
                    (true, None) => first = Some(corr_id),
                    (true, Some(old)) => {
                        let late = ShellMessage::Stdout {
                            data_b64: "bGF0ZQ==".into(),
                        };
                        let old_end = ShellMessage::Exited { code: 137 };
                        let end = ShellMessage::Exited { code: 0 };
                        for frame in [
                            encode_frame(old, 0, &late),
                            encode_frame(old, FLAG_TERMINAL, &old_end),
                            encode_frame(corr_id, FLAG_TERMINAL, &end),
                        ] {
                            writer.write_all(&frame.unwrap()).unwrap();
                        }
                    }
                }
            }
        });
        let sleep = ShellMessage::Request {
            cmd: "/bin/sleep".into(),
            args: vec!["30".into()],
            env: vec![],
            cwd: None,
            tty: false,
            rows: 24,
            cols: 80,
        };
        let (abandoned, old_id) = open_session(&sock, &sleep).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(abandoned);
        // Past any hold a slot once had after a killed session.
        tokio::time::sleep(Duration::from_millis(600)).await;

        let (mut next, new_id) = open_session(&sock, &sleep).await;
        let frame = tokio::time::timeout(Duration::from_secs(2), read_frame(&mut next))
            .await
            .expect("no reply")
            .unwrap()
            .expect("frame");
        let (corr_id, flags, msg) =
            iii_supervisor::shell_protocol::decode_frame_body(&frame[4..]).unwrap();
        assert_eq!((corr_id, msg), (new_id, ShellMessage::Exited { code: 0 }));
        assert!(flags & FLAG_TERMINAL != 0);
        assert_ne!(
            new_id, old_id,
            "the next client got the abandoned session's id"
        );
        let killed = seen_rx
            .try_iter()
            .any(|(id, msg)| id == old_id && msg == ShellMessage::Signal { signal: 9 });
        assert!(killed, "no SIGKILL for the abandoned session");
    }

    /// A new range skips the ranges of live clients and of sessions the
    /// guest may still be ending, and comes back once the guest ends them.
    #[test]
    fn ranges_skip_live_clients_and_lingering_sessions() {
        let clients = ClientTable::default();
        assert_eq!(claim_range(&clients), Some(0));
        release_range(&clients, 0, vec![5]);
        clients.lock().unwrap().live.insert(2 * ID_RANGE_STEP);
        clients.lock().unwrap().next_range = 0;
        assert_eq!(claim_range(&clients), Some(ID_RANGE_STEP));
        assert_eq!(claim_range(&clients), Some(3 * ID_RANGE_STEP));
        clients.lock().unwrap().lingering.remove(&5);
        clients.lock().unwrap().next_range = 0;
        assert_eq!(claim_range(&clients), Some(0));
    }
}
