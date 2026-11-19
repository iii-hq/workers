// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! Child-issued pidfd, never pidfd_open of a post-spawn diagnostic PID.
use super::{retirement::WriterRetirement, storage::*};
use std::{
    fmt, io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    process::ExitStatus,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeSignal {
    Terminate,
    Kill,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimePhase {
    Running,
    Reaped,
    WaitFailed,
    Abandoned,
    RetirementFailed,
    Retired,
}
#[derive(Clone, Debug)]
pub(crate) struct RuntimeFailure {
    kind: io::ErrorKind,
    raw: Option<i32>,
    message: Arc<str>,
}
impl RuntimeFailure {
    pub(super) fn capture(error: &io::Error) -> Self {
        let text = error.to_string();
        let end = text.char_indices().nth(2048).map_or(text.len(), |(i, _)| i);
        Self {
            kind: error.kind(),
            raw: error.raw_os_error(),
            message: Arc::from(&text[..end]),
        }
    }
    pub(crate) fn kind(&self) -> io::ErrorKind {
        self.kind
    }
    pub(crate) fn raw_os_error(&self) -> Option<i32> {
        self.raw
    }
    pub(crate) fn to_io_error(&self) -> io::Error {
        io::Error::new(self.kind, self.to_string())
    }
}
impl fmt::Display for RuntimeFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (os={:?})", self.message, self.raw)
    }
}
impl std::error::Error for RuntimeFailure {}
#[derive(Clone)]
pub(crate) struct RuntimeObservation {
    phase: RuntimePhase,
    status: Option<ExitStatus>,
    failure: Option<RuntimeFailure>,
    retirement: Option<WriterRetirement>,
}
impl RuntimeObservation {
    pub(crate) fn phase(&self) -> RuntimePhase {
        self.phase
    }
    pub(crate) fn os_status(&self) -> Option<ExitStatus> {
        self.status
    }
    pub(crate) fn failure(&self) -> Option<&RuntimeFailure> {
        self.failure.as_ref()
    }
    pub(crate) fn retirement(&self) -> Option<&WriterRetirement> {
        self.retirement.as_ref()
    }
    pub(crate) fn is_terminal(&self) -> bool {
        !matches!(self.phase, RuntimePhase::Running | RuntimePhase::Reaped)
    }
}
struct ProcessState {
    pidfd: Option<OwnedFd>,
    signalable: bool,
    observation: RuntimeObservation,
}
struct Shared {
    state: Mutex<ProcessState>,
    events: watch::Sender<RuntimeObservation>,
}
/// The diagnostic PID is intentionally not accepted by any method.
#[derive(Clone)]
pub(crate) struct RuntimeControl(Arc<Shared>);
/// No receiver borrow escapes: consumers cannot hold a watch read lock while
/// requesting a signal or waiting for the next core transition.
pub(crate) struct RuntimeObserver {
    control: RuntimeControl,
    events: watch::Receiver<RuntimeObservation>,
}
impl RuntimeObserver {
    pub(crate) fn observation(&self) -> RuntimeObservation {
        self.control.observation()
    }
    pub(crate) async fn changed(&mut self) -> RuntimeObservation {
        let _ = self.events.changed().await;
        self.control.observation()
    }
}
impl RuntimeControl {
    pub(super) fn new(pidfd: Option<OwnedFd>, acquisition: Option<&io::Error>) -> Self {
        let observation = RuntimeObservation {
            phase: RuntimePhase::Running,
            status: None,
            failure: acquisition.map(RuntimeFailure::capture),
            retirement: None,
        };
        let (events, _) = watch::channel(observation.clone());
        Self(Arc::new(Shared {
            state: Mutex::new(ProcessState {
                signalable: pidfd.is_some(),
                pidfd,
                observation,
            }),
            events,
        }))
    }
    // This mutex only guards bounded local state plus one nonblocking signal
    // syscall. Never held across Child.wait, catalog I/O, callbacks or await.
    fn state(&self) -> std::sync::MutexGuard<'_, ProcessState> {
        self.0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub(crate) fn observation(&self) -> RuntimeObservation {
        self.state().observation.clone()
    }
    pub(crate) fn subscribe(&self) -> RuntimeObserver {
        RuntimeObserver {
            control: self.clone(),
            events: self.0.events.subscribe(),
        }
    }
    pub(crate) async fn terminal(&self) -> RuntimeObservation {
        let mut observer = self.subscribe();
        loop {
            let current = observer.observation();
            if current.is_terminal() {
                return current;
            }
            let next = observer.changed().await;
            if next.is_terminal() {
                return next;
            }
        }
    }
    pub(crate) fn signal(&self, signal: RuntimeSignal) -> io::Result<()> {
        let state = self.state();
        if !state.signalable {
            return Err(invalid(
                "child is reaped or no stable process handle exists",
            ));
        }
        let fd = state
            .pidfd
            .as_ref()
            .ok_or_else(|| invalid("missing stable process handle"))?;
        #[cfg(target_os = "linux")]
        {
            let number = match signal {
                RuntimeSignal::Terminate => libc::SIGTERM,
                RuntimeSignal::Kill => libc::SIGKILL,
            };
            if unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    fd.as_raw_fd(),
                    number,
                    std::ptr::null::<libc::siginfo_t>(),
                    0,
                )
            } < 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (fd, signal);
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "runtime signaling requires Linux pidfd",
            ))
        }
    }
    pub(super) fn reaped(&self, status: ExitStatus) {
        let mut state = self.state();
        state.signalable = false;
        state.pidfd.take();
        state.observation.phase = RuntimePhase::Reaped;
        state.observation.status = Some(status);
        self.0.events.send_replace(state.observation.clone());
    }
    pub(super) fn abandoned_reaped(&self, status: ExitStatus) {
        let mut state = self.state();
        state.signalable = false;
        state.pidfd.take();
        state.observation.status = Some(status);
        if !state.observation.is_terminal() {
            state.observation.phase = RuntimePhase::Abandoned;
        }
        self.0.events.send_replace(state.observation.clone());
    }
    pub(super) fn failed(&self, phase: RuntimePhase, error: &io::Error) {
        let mut state = self.state();
        state.observation.phase = phase;
        state.observation.failure = Some(RuntimeFailure::capture(error));
        self.0.events.send_replace(state.observation.clone());
    }
    pub(super) fn retired(&self, receipt: WriterRetirement) {
        let mut state = self.state();
        state.observation.phase = RuntimePhase::Retired;
        state.observation.retirement = Some(receipt);
        state.observation.failure = None;
        self.0.events.send_replace(state.observation.clone());
    }
}
/// socketpair is used only as a private pre-exec descriptor handoff, not an
/// endpoint or network protocol. Child obtains a pidfd for ITSELF while alive.
pub(super) struct ProcessHandoff {
    parent: OwnedFd,
    child: OwnedFd,
}
impl ProcessHandoff {
    pub(super) fn new() -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let mut pair = [-1; 2];
            syscall(unsafe {
                libc::socketpair(
                    libc::AF_UNIX,
                    libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                    0,
                    pair.as_mut_ptr(),
                )
            })?;
            Ok(Self {
                parent: unsafe { OwnedFd::from_raw_fd(pair[0]) },
                child: unsafe { OwnedFd::from_raw_fd(pair[1]) },
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "runtime process identity requires Linux pidfd",
            ))
        }
    }
    pub(super) fn install(&self, command: &mut tokio::process::Command) {
        let child = self.child.as_raw_fd();
        let parent = self.parent.as_raw_fd();
        // SAFETY: only async-signal-safe syscalls and stack data after fork. The
        // pidfd is opened for getpid() in the not-yet-exec'd child, BEFORE any
        // parent waiter or Tokio orphan reaper can observe child exit.
        unsafe {
            command.pre_exec(move || {
                libc::close(parent);
                #[cfg(target_os = "linux")]
                {
                    let fd = libc::syscall(libc::SYS_pidfd_open, libc::getpid(), 0) as libc::c_int;
                    if fd < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    let sent = send_fd(child, fd);
                    libc::close(fd);
                    libc::close(child);
                    sent
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = child;
                    Err(io::Error::from_raw_os_error(libc::ENOSYS))
                }
            });
        }
    }
    pub(super) fn receive(self) -> io::Result<OwnedFd> {
        #[cfg(target_os = "linux")]
        {
            drop(self.child);
            receive_fd(self.parent.as_raw_fd())
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "runtime pidfd handoff requires Linux",
            ))
        }
    }
}
#[cfg(target_os = "linux")]
#[repr(C)]
struct Ancillary {
    header: libc::cmsghdr,
    fd: libc::c_int,
    padding: libc::c_int,
}
#[cfg(target_os = "linux")]
fn send_fd(socket: i32, fd: i32) -> io::Result<()> {
    let mut byte = 0x46u8;
    let mut iov = libc::iovec {
        iov_base: (&mut byte as *mut u8).cast(),
        iov_len: 1,
    };
    let mut ancillary: Ancillary = unsafe { std::mem::zeroed() };
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = (&mut ancillary as *mut Ancillary).cast();
    msg.msg_controllen = unsafe { libc::CMSG_SPACE(std::mem::size_of::<i32>() as u32) } as usize;
    unsafe {
        let header = libc::CMSG_FIRSTHDR(&msg);
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<i32>() as u32) as usize;
        std::ptr::write_unaligned(libc::CMSG_DATA(header).cast::<i32>(), fd);
    }
    loop {
        let n = unsafe { libc::sendmsg(socket, &msg, libc::MSG_NOSIGNAL) };
        if n == 1 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}
#[cfg(target_os = "linux")]
fn receive_fd(socket: i32) -> io::Result<OwnedFd> {
    let mut byte = 0u8;
    let mut iov = libc::iovec {
        iov_base: (&mut byte as *mut u8).cast(),
        iov_len: 1,
    };
    let mut ancillary: Ancillary = unsafe { std::mem::zeroed() };
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = (&mut ancillary as *mut Ancillary).cast();
    msg.msg_controllen = unsafe { libc::CMSG_SPACE(std::mem::size_of::<i32>() as u32) } as usize;
    let n = unsafe { libc::recvmsg(socket, &mut msg, libc::MSG_CMSG_CLOEXEC) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    let header = unsafe { libc::CMSG_FIRSTHDR(&msg) };
    if header.is_null() {
        return Err(invalid("child did not transfer its stable process handle"));
    }
    if unsafe {
        (*header).cmsg_level != libc::SOL_SOCKET
            || (*header).cmsg_type != libc::SCM_RIGHTS
            || (*header).cmsg_len != libc::CMSG_LEN(std::mem::size_of::<i32>() as u32) as usize
    } {
        return Err(invalid("invalid process handle handoff"));
    }
    let raw = unsafe { std::ptr::read_unaligned(libc::CMSG_DATA(header).cast::<i32>()) };
    if raw < 0 {
        return Err(invalid("invalid received process descriptor"));
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    if n != 1 || byte != 0x46 || msg.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) != 0 {
        return Err(invalid("truncated process handle handoff"));
    }
    Ok(fd)
}
