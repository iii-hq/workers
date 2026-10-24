// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Hidden `__vm-boot` subcommand -- boots a libkrun microVM.
//!
//! Runs in a separate process (spawned via `current_exe() __vm-boot`)
//! for crash isolation. If libkrun segfaults, only this child dies.
//!
//! Uses msb_krun VmBuilder for type-safe VM configuration.

/// Arguments for the `__vm-boot` hidden subcommand.
#[derive(clap::Args, Debug)]
pub struct VmBootArgs {
    /// Path to the guest rootfs directory (the boot trampoline; in overlay
    /// mode this is a minimal dir serving only the injected /init.krun).
    #[arg(long)]
    pub rootfs: String,

    /// Overlay mode: host path to the prebuilt read-only base erofs
    /// (attached as the overlay lower, /dev/vda). Built host-side by the
    /// start path; absent => legacy per-worker-rootfs boot.
    #[arg(long)]
    pub rootfs_lower: Option<String>,

    /// Rootfs mode signalled by the start path: "overlay" or "" (legacy).
    #[arg(long, default_value = "")]
    pub rootfs_mode: String,

    /// Overlay mode: host path to this worker's writable upper image
    /// (a pre-formatted ext4 materialized from the embedded golden image,
    /// attached as /dev/vdb). Absent => iii-init uses a non-persistent
    /// tmpfs upper (dev builds without the embedded golden).
    #[arg(long)]
    pub rootfs_upper: Option<String>,

    /// Serve `--rootfs` read-only over virtio-fs: every guest write through
    /// `/dev/root` fails with EROFS. The sandbox adapter sets this for its
    /// per-sandbox trampoline; worker VMs leave it off.
    #[arg(long)]
    pub rootfs_readonly: bool,

    /// Executable path inside the guest
    #[arg(long)]
    pub exec: String,

    /// Arguments to pass to the guest executable
    #[arg(long, allow_hyphen_values = true)]
    pub arg: Vec<String>,

    /// Working directory inside the guest
    #[arg(long, default_value = "/")]
    pub workdir: String,

    /// Number of vCPUs
    #[arg(long, default_value = "2")]
    pub vcpus: u32,

    /// RAM in MiB
    #[arg(long, default_value = "2048")]
    pub ram: u32,

    /// Volume mounts (host_path:guest_path)
    #[arg(long)]
    pub mount: Vec<String>,

    /// Environment variables (KEY=VALUE). Visible to every local user in
    /// `ps`; launchers pass the worker env with `--env-stdin` instead.
    #[arg(long)]
    pub env: Vec<String>,

    /// Read more environment variables from stdin, as one
    /// `iii_supervisor::env_channel` frame. Keeps the worker env
    /// (credentials included) out of this process's argv, where `ps` shows
    /// it to every local user (MOT-5492). Applied after `--env`.
    #[arg(long)]
    pub env_stdin: bool,

    /// Network slot for IP/MAC address derivation (0-65535)
    #[arg(long, default_value = "0")]
    pub slot: u64,

    /// Enable SMT / hyperthreading in the guest. Default off — matches
    /// Firecracker's conservative default and avoids noisy-neighbour
    /// effects between co-tenant workers. Flip on to squeeze more
    /// throughput on Intel hosts where the perf delta is measurable.
    #[arg(long, default_value = "false")]
    pub hyperthreading: bool,

    /// Enable nested virtualization. Default off — a microworker VM has
    /// no need to run its own hypervisor, and enabling nested-KVM is a
    /// measurable perf hit. Flip on only for CI/test scenarios that
    /// explicitly boot a VM inside the worker.
    #[arg(long, default_value = "false")]
    pub nested_virt: bool,

    /// DAX window size (MiB) for every virtiofs mount. 0 means
    /// "take the msb_krun default". Larger windows cut syscall
    /// roundtrips on read-heavy mounts (node_modules, .venv,
    /// site-packages); cost is guest virtual address space and a
    /// small amount of host RAM per mount. Safe starting point:
    /// 256 for dev workflows with large dep trees.
    #[arg(long, default_value = "0")]
    pub virtiofs_shm_size_mib: u32,

    /// Soft / hard limit for `RLIMIT_NOFILE` applied to the guest
    /// worker process via libkrun's `KRUN_RLIMITS`. Complements the
    /// guest-side raise in `iii-init::rlimit::raise_nofile` — setting
    /// it here means every fd opened by `init.krun` BEFORE iii-init
    /// runs (dynamic loader, early allocations) also sees the raised
    /// limit. 0 means "don't set" (fall back to iii-init's raise).
    #[arg(long, default_value = "65536")]
    pub nofile_limit: u64,

    /// Unix socket path where this `__vm-boot` process will listen for
    /// exec and fs clients (`sandbox::exec`, `sandbox::fs::*`). Sessions
    /// are multiplexed: many concurrent requests on one virtio-console
    /// port.
    ///
    /// When set, `__vm-boot` creates a `socketpair(AF_UNIX)`,
    /// wires one end into the VM as a named virtio-console port
    /// (`iii.exec`), and spawns the async [`crate::cli::shell_relay`]
    /// task on the existing tokio runtime to route frames between
    /// connecting clients and the VM. The in-VM `iii-init` shell
    /// dispatcher reads/writes the other end.
    ///
    /// When absent, the VM boots without an exec port, and an exec
    /// client fails on the missing socket rather than hanging.
    #[arg(long)]
    pub shell_sock: Option<String>,

    /// Host source directory whose changes are forwarded into the guest over
    /// `iii.source-sync`. Local-path workers set this; immutable bundles and
    /// ordinary volume mounts do not.
    #[arg(long)]
    pub source_sync: Option<String>,

    /// Enable network egress for the guest. When false, the smoltcp
    /// userspace TCP/IP stack is not initialized and no virtio-net
    /// device is attached, so the VM has no network interface — useful
    /// for fully-isolated build steps. When true, the host runs an
    /// `iii_network::SmoltcpNetwork` instance and proxies guest TCP
    /// connections to the host (gateway IP rewrites to 127.0.0.1, see
    /// `iii-network/src/proxy.rs`); guest-side `iii-init` reads
    /// `III_INIT_IP` / `III_INIT_GW` / `III_INIT_CIDR` and configures
    /// `eth0`. The host-side adapter (`sandbox::create`) passes
    /// `--network` only when the request's `network` field is true.
    #[arg(long, default_value = "false")]
    pub network: bool,

    /// Network-activity beacon file (only meaningful with `--network`).
    /// The smoltcp stack refreshes this file's mtime whenever guest
    /// payload is relayed (TCP data, UDP datagrams, DNS queries), so the
    /// sandbox daemon's idle reaper — a separate process with no other
    /// view into this one — can tell a network-serving sandbox from an
    /// idle one. Absent => no beacon; the sandbox is reaped on
    /// exec-idleness alone, as before.
    #[arg(long)]
    pub net_activity_file: Option<String>,
}

/// One `--mount host:guest` CLI arg, expanded into the virtiofs attach plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtiofsMountEntry {
    pub tag: String,
    pub host_path: String,
    pub guest_path: String,
}

/// Output of [`build_virtiofs_mount_plan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtiofsMountPlan {
    /// Virtio-fs attach entries, in CLI arg order. Each gets a `virtiofs_N` tag.
    pub entries: Vec<VirtiofsMountEntry>,
    /// Value of `III_VIRTIOFS_MOUNTS` to pass into the guest env:
    /// `tag1=/guest/path1;tag2=/guest/path2`.
    pub env_var: String,
}

/// Parse `--mount host:guest` CLI args into a virtiofs attach plan and the
/// matching `III_VIRTIOFS_MOUNTS` env string the guest (iii-init) will consume.
///
/// Returns `Err` on the first malformed entry so a bad CLI arg fails the VM
/// boot instead of producing a partial attach plan.
pub fn build_virtiofs_mount_plan(mounts: &[String]) -> Result<VirtiofsMountPlan, String> {
    let mut entries = Vec::with_capacity(mounts.len());
    let mut env_var = String::new();
    for (i, mount_str) in mounts.iter().enumerate() {
        let (host_path, guest_path) = match mount_str.split_once(':') {
            Some((h, g)) if !h.is_empty() && !g.is_empty() => (h.to_string(), g.to_string()),
            _ => {
                return Err(format!(
                    "Invalid mount format '{}'. Expected host:guest",
                    mount_str
                ));
            }
        };
        let tag = format!("virtiofs_{}", i);
        if !env_var.is_empty() {
            env_var.push(';');
        }
        env_var.push_str(&tag);
        env_var.push('=');
        env_var.push_str(&guest_path);
        entries.push(VirtiofsMountEntry {
            tag,
            host_path,
            guest_path,
        });
    }
    Ok(VirtiofsMountPlan { entries, env_var })
}

/// Compose the full libkrunfw file path from the resolved directory and platform filename.
pub fn resolve_krunfw_file_path() -> Option<std::path::PathBuf> {
    let dir = crate::cli::firmware::resolve::resolve_libkrunfw_dir()?;
    let filename = crate::cli::firmware::constants::libkrunfw_filename();
    let file_path = dir.join(&filename);
    if file_path.exists() {
        Some(file_path)
    } else {
        None
    }
}

/// Pre-flight check for KVM availability on Linux.
#[cfg(target_os = "linux")]
fn check_kvm_available() -> Result<(), String> {
    check_kvm_at_path(std::path::Path::new("/dev/kvm"))
}

#[cfg(target_os = "linux")]
fn check_kvm_at_path(kvm: &std::path::Path) -> Result<(), String> {
    if !kvm.exists() {
        return Err("KVM not available -- /dev/kvm does not exist. \
             Ensure KVM is enabled in your kernel and loaded (modprobe kvm_intel or kvm_amd). \
             This can also occur on Windows with WSL. \
             See https://iii.dev/docs/troubleshooting#kvm-not-accessible"
            .to_string());
    }
    match std::fs::File::options().read(true).write(true).open(kvm) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Err(
            "KVM not accessible -- /dev/kvm exists but current user lacks permission. \
             Add your user to the 'kvm' group: sudo usermod -aG kvm $USER. \
             See https://iii.dev/docs/troubleshooting#kvm-not-accessible"
                .to_string(),
        ),
        Err(e) => Err(format!("KVM check failed: {}", e)),
    }
}

/// The guest's `/dev/root` share: `--rootfs` served by PassthroughFs,
/// read-only under `--rootfs-readonly` (the sandbox trampoline).
pub(crate) fn root_share(args: &VmBootArgs) -> Result<iii_filesystem::PassthroughFs, String> {
    iii_filesystem::PassthroughFs::builder()
        .root_dir(&args.rootfs)
        .read_only(args.rootfs_readonly)
        .build()
        .map_err(|e| format!("PassthroughFs failed for '{}': {}", args.rootfs, e))
}

/// Raise the process fd limit (RLIMIT_NOFILE) to accommodate PassthroughFs.
fn raise_fd_limit() {
    use nix::libc;
    let mut rlim: libc::rlimit = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) } == 0 {
        let target = rlim.rlim_max.min(1_048_576);
        if rlim.rlim_cur < target {
            rlim.rlim_cur = target;
            unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &rlim) };
        }
    }
}

pub fn shell_quote(s: &str) -> String {
    if s.chars().all(|c| {
        c.is_alphanumeric() || c == '-' || c == '_' || c == '/' || c == '.' || c == ':' || c == '='
    }) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

pub fn build_worker_cmd(exec: &str, args: &[String]) -> String {
    if args.is_empty() {
        shell_quote(exec)
    } else {
        let mut parts = vec![shell_quote(exec)];
        for arg in args {
            parts.push(shell_quote(arg));
        }
        parts.join(" ")
    }
}

/// Rewrite localhost/loopback URLs to use the given gateway IP.
/// Used by the VM boot process to redirect traffic into the guest network.
pub fn rewrite_localhost(s: &str, gateway_ip: &str) -> String {
    s.replace("://localhost:", &format!("://{}:", gateway_ip))
        .replace("://127.0.0.1:", &format!("://{}:", gateway_ip))
}

/// Default PATH for the guest VM when neither the caller nor the OCI
/// image config provides one. Mirrors the value every standard
/// Debian/Ubuntu image (and most other distros) ships in
/// `/etc/profile`.
///
/// Why this matters: libkrun starts iii-init (PID 1) with whatever
/// env we pass to `set_env`, and only that. When `args.env` doesn't
/// carry `PATH`, PID 1 has none — and neither does any process it
/// spawns (the worker boot script, the shell dispatcher, `sandbox::exec`
/// children). dash's compile-time default fills `$PATH`
/// internally so `node` typed at a shell prompt still works, but
/// **dash does not mark that internal default as exported**. So a
/// `#!/usr/bin/env <prog>` shebang re-execs through `/usr/bin/env`,
/// which inherits the empty env, falls back to libc's
/// `_PATH_DEFPATH` (`/usr/bin:/bin`), and can't find anything in
/// `/usr/local/bin` — breaking npm/npx/yarn/pip/etc.
///
/// The caller-supplied env wins: if the worker env already carries
/// `PATH`, this constant is not used.
pub const DEFAULT_GUEST_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// Conditionally rewrite localhost/loopback URLs.
///
/// `None` means networking is disabled — no virtio-net device is attached,
/// so there is no gateway and `localhost`/`127.0.0.1` resolve correctly via
/// the guest's loopback. The rewrite is skipped to avoid mangling URLs into
/// `://:PORT`.
///
/// `Some(gw)` mirrors the original behaviour: replace `://localhost:` and
/// `://127.0.0.1:` with `://gw:`.
pub fn maybe_rewrite_localhost(s: &str, gateway_ip: Option<&str>) -> String {
    match gateway_ip {
        Some(gw) => rewrite_localhost(s, gw),
        None => s.to_string(),
    }
}

/// Create a unix stream `socketpair` for one virtio-console port. The guest
/// fd will be handed to `ConsoleBuilder::port` and must remain open for
/// the lifetime of the VM; we deliberately `forget` the owned wrapper
/// and return the raw fd number so it isn't closed when this function
/// returns. The host end is kept as a `UnixStream` in the caller.
///
/// Rationale: libkrun takes ownership of the guest fd internally at
/// VM build time. Dropping our Rust-owned wrapper after handing the
/// raw fd to libkrun would close the fd underneath the VM; forgetting
/// keeps it alive. Closing happens on process exit, which is exactly
/// when the VM goes down — correct.
#[cfg(unix)]
fn setup_port_socketpair() -> Result<(std::os::unix::net::UnixStream, i32), String> {
    use std::os::fd::{AsRawFd, IntoRawFd};
    use std::os::unix::net::UnixStream;

    let (host_end, guest_end) =
        UnixStream::pair().map_err(|e| format!("console port socketpair: {e}"))?;

    // Clear CLOEXEC on the guest fd so it remains open through any
    // libkrun internal fork/exec pathway. (On most platforms
    // UnixStream::pair sets FD_CLOEXEC; libkrun consumes the fd
    // in-process so CLOEXEC doesn't matter today, but this is cheap
    // insurance against future changes.)
    unsafe {
        let fd = guest_end.as_raw_fd();
        let flags = nix::libc::fcntl(fd, nix::libc::F_GETFD);
        if flags >= 0 {
            nix::libc::fcntl(fd, nix::libc::F_SETFD, flags & !nix::libc::FD_CLOEXEC);
        }
    }

    let guest_fd = guest_end.into_raw_fd();
    Ok((host_end, guest_fd))
}

/// Reads the `--env-stdin` frame, then points stdin at /dev/null so the
/// detached VM does not keep the launcher's pipe open. A missing or
/// truncated frame (launcher died mid-write) is an error: booting a worker
/// without its env fails later and less clearly.
fn read_env_stdin() -> Result<Vec<(String, String)>, String> {
    use std::os::fd::AsRawFd;

    let pairs = iii_supervisor::env_channel::read_frame(&mut std::io::stdin().lock())
        .map_err(|e| format!("--env-stdin: cannot read the worker environment: {e}"))?;
    if let Ok(devnull) = std::fs::File::open("/dev/null") {
        // SAFETY: dup2 onto fd 0 with a valid open fd; `devnull` closes its
        // own copy on drop.
        unsafe {
            nix::libc::dup2(devnull.as_raw_fd(), 0);
        }
    }
    Ok(pairs)
}

/// Writes the env frame into the host end of the `iii.env` port on its own
/// thread (a socketpair buffer can be smaller than the frame, and the guest
/// only drains it once iii-init runs), then keeps that end open for the VM's
/// lifetime: closing it would hang the port up under the guest.
#[cfg(unix)]
fn spawn_env_writer(
    mut host_end: std::os::unix::net::UnixStream,
    frame: Vec<u8>,
) -> Result<(), String> {
    use std::io::{Read, Write};

    std::thread::Builder::new()
        .name("iii-env-port".to_string())
        .spawn(move || {
            if let Err(e) = host_end.write_all(&frame) {
                let _ = writeln!(
                    std::io::stderr(),
                    "error: cannot send the worker environment to the VM: {e}"
                );
                return;
            }
            drop(frame);
            let mut sink = [0u8; 64];
            while matches!(host_end.read(&mut sink), Ok(n) if n > 0) {}
        })
        .map(|_| ())
        .map_err(|e| format!("cannot start the env writer thread: {e}"))
}

/// Budget for what iii adds to the kernel command line. msb_krun's limit is
/// 16 KiB for the whole line, and it unwraps the insert (`TooLarge` panics).
/// Its own part (default cmdline, root/init, KRUN_INIT, KRUN_RLIMITS,
/// KRUN_BOOT_START_NS, the `--` epilog) is about 260 bytes; 1 KiB is left
/// for it.
const CMDLINE_ENV_BUDGET: usize = 15 * 1024;

/// Variables iii-init owns that a worker env must never set: the worker
/// command and the port announcement. Compose forwards the whole machine
/// env, so a stray one there would otherwise replace them (on the legacy
/// command line the last duplicate wins).
const RESERVED_WORKER_ENV: &[&str] = &["III_WORKER_CMD", iii_supervisor::env_channel::ENV_PORT_ENV];

/// Where each variable reaches the guest.
#[derive(Debug, PartialEq)]
struct VmEnvPlan {
    /// Kernel command line: iii-init's plumbing, plus the worker command and
    /// env on the legacy path.
    cmdline: Vec<(String, String)>,
    /// The `iii.env` frame (port path only): the worker env, then the worker
    /// command last.
    frame: Option<Vec<(String, String)>>,
}

/// Splits the guest environment between the kernel command line and the
/// `iii.env` frame (MOT-5493). `plumbing` is iii-init's own configuration,
/// in order; `worker_pairs` is the worker env, already localhost-rewritten.
fn split_vm_env(
    plumbing: Vec<(String, String)>,
    worker_pairs: Vec<(String, String)>,
    worker_cmd: &str,
    use_env_port: bool,
) -> VmEnvPlan {
    let worker_pairs: Vec<(String, String)> = worker_pairs
        .into_iter()
        .filter(|(key, _)| !RESERVED_WORKER_ENV.contains(&key.as_str()))
        .collect();
    // Fallback PATH for rootfs caches that pre-date the
    // `.oci-config.json` write step (`oci.rs:604`) — `read_oci_env`
    // returns [] on those, so PATH never makes it into the worker env,
    // and shebang scripts like `#!/usr/bin/env node` then fail to
    // resolve binaries that live in /usr/local/bin. An explicit
    // `PATH` (or an OCI image that does ship PATH) always wins.
    let needs_path = !worker_pairs.iter().any(|(key, _)| key == "PATH");
    let command = ("III_WORKER_CMD".to_string(), worker_cmd.to_string());

    let mut cmdline = Vec::new();
    let frame = if use_env_port {
        cmdline.extend(plumbing);
        cmdline.push((
            iii_supervisor::env_channel::ENV_PORT_ENV.to_string(),
            iii_supervisor::env_channel::ENV_PORT_NAME.to_string(),
        ));
        let mut frame = worker_pairs;
        frame.push(command);
        Some(frame)
    } else {
        // The order the exec closure always used.
        cmdline.push(command);
        cmdline.extend(plumbing);
        cmdline.extend(worker_pairs);
        None
    };
    if needs_path {
        cmdline.push(("PATH".to_string(), DEFAULT_GUEST_PATH.to_string()));
    }
    VmEnvPlan { cmdline, frame }
}

/// Checks everything iii puts on the kernel command line before msb_krun
/// unwraps the insert. msb_krun writes each pair as ` KEY="VALUE"` and only
/// accepts printable ASCII: anything else panics the boot (`InvalidAscii`,
/// MOT-5493), and an embedded `"` silently splits the value. Names the key,
/// never the value.
fn validate_cmdline_env(env: &[(String, String)]) -> Result<(), String> {
    let carriable = |s: &str| s.chars().all(|c| matches!(c, ' '..='~') && c != '"');
    for (key, value) in env {
        if !carriable(key) || key.contains(' ') {
            return Err(format!(
                "{key:?} cannot be passed to the VM: the kernel command line only carries \
                 printable ASCII names without spaces or quotes"
            ));
        }
        if !carriable(value) {
            return Err(format!(
                "the value of {key} cannot be passed to the VM: the kernel command line \
                 only carries printable ASCII without '\"'"
            ));
        }
    }
    let total: usize = env.iter().map(|(k, v)| k.len() + v.len() + 4).sum();
    if total > CMDLINE_ENV_BUDGET {
        return Err(format!(
            "iii's part of the VM kernel command line is {total} bytes, over the \
             {CMDLINE_ENV_BUDGET}-byte budget"
        ));
    }
    Ok(())
}

/// Boot the VM. Called from `main()` when `__vm-boot` is parsed.
/// This function does NOT return -- `krun_start_enter` replaces the process.
pub fn run(args: &VmBootArgs) -> ! {
    // Drain the launcher's env pipe before anything else can make us exit:
    // a launcher blocked writing a large frame must not outlive us.
    let stdin_env = if args.env_stdin {
        match read_env_stdin() {
            Ok(pairs) => pairs,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
    } else {
        Vec::new()
    };

    if !std::path::Path::new(&args.rootfs).exists() {
        eprintln!("error: rootfs path does not exist: {}", args.rootfs);
        std::process::exit(1);
    }

    match boot_vm(args, stdin_env) {
        Ok(infallible) => match infallible {},
        Err(e) => {
            eprintln!("error: VM execution failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn boot_vm(
    args: &VmBootArgs,
    stdin_env: Vec<(String, String)>,
) -> Result<std::convert::Infallible, String> {
    use msb_krun::VmBuilder;

    #[cfg(target_os = "linux")]
    check_kvm_available()?;

    raise_fd_limit();

    // Worker env: argv `--env` pairs (manual runs, tests) plus the frame a
    // launcher piped in with `--env-stdin`. Later entries win.
    let worker_env = args
        .env
        .iter()
        .filter_map(|kv| kv.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .chain(stdin_env);

    // Pre-boot validation: ensure init binary is available either embedded or on-disk
    if !iii_filesystem::init::has_init() {
        let init_on_disk = std::path::Path::new(&args.rootfs).join("init.krun");
        if !init_on_disk.exists() {
            return Err(format!(
                "No init binary available. /init.krun not found in rootfs '{}' \
                 and no init binary is embedded in this build.\n\
                 Hint: rebuild sandbox-builtin with --features embed-init.",
                args.rootfs
            ));
        }
    }

    if args.vcpus > u8::MAX as u32 {
        return Err(format!(
            "vcpus {} exceeds maximum {} for VmBuilder",
            args.vcpus,
            u8::MAX
        ));
    }

    let passthrough_fs = root_share(args)?;

    let worker_cmd = build_worker_cmd(&args.exec, &args.arg);

    let hyperthreading = args.hyperthreading;
    let nested_virt = args.nested_virt;
    let mut builder = VmBuilder::new()
        .machine(|m| {
            m.vcpus(args.vcpus as u8)
                .memory_mib(args.ram as usize)
                .hyperthreading(hyperthreading)
                .nested_virt(nested_virt)
        })
        .kernel(|k| {
            let k = match resolve_krunfw_file_path() {
                Some(path) => k.krunfw_path(&path),
                None => k,
            };
            k.init_path("/init.krun")
        })
        .fs(move |fs| fs.tag("/dev/root").custom(Box::new(passthrough_fs)));

    let mount_plan = build_virtiofs_mount_plan(&args.mount)?;
    let virtiofs_mount_env = mount_plan.env_var.clone();
    // Per-mount DAX window. 0 (the CLI default) skips the call so
    // msb_krun's built-in default applies; any positive value is
    // converted to bytes and set on every virtiofs attach.
    let shm_bytes_per_mount: usize =
        (args.virtiofs_shm_size_mib as usize).saturating_mul(1024 * 1024);
    for entry in mount_plan.entries {
        let tag = entry.tag.clone();
        let host_path = entry.host_path.clone();
        builder = builder.fs(move |fs| {
            let mut fs = fs.tag(&tag);
            if shm_bytes_per_mount > 0 {
                fs = fs.shm_size(shm_bytes_per_mount);
            }
            fs.path(&host_path)
        });
    }

    // Overlay mode is decided by the START path (which evaluated the feature
    // flag + the embedded-init capability handshake — same binary, so the
    // decision is authoritative) and passed down as --rootfs-mode + the
    // prebuilt --rootfs-lower erofs. We attach it here; the erofs is
    // built host-side in the start path, not in this detached child.
    let overlay_mode = args.rootfs_mode == "overlay" && args.rootfs_lower.is_some();
    if overlay_mode {
        let lower_img = args
            .rootfs_lower
            .clone()
            .expect("overlay_mode implies --rootfs-lower");
        eprintln!("iii: overlay base erofs -> /dev/vda: {lower_img}");
        // Disk attach ORDER is the guest device order: the lower MUST be the
        // first disk (/dev/vda) and the upper the second (/dev/vdb). iii-init
        // reads III_BLOCK_ROOT_LOWER=/dev/vda and III_BLOCK_ROOT_UPPER=/dev/vdb.
        builder = builder.disk(move |d| {
            d.path(&lower_img)
                .format(msb_krun::DiskImageFormat::Raw)
                .read_only(true)
        });
        if let Some(upper) = args.rootfs_upper.clone() {
            // Advisory flock guards the rw ext4 upper against a SECOND VM
            // attaching it concurrently (ext4 has no multi-mount protection →
            // silent corruption). The adapter stops the old VM before a
            // restart; this catches a concurrent same-worker start. The lock
            // fd is leaked so the lock is held for the VM's whole lifetime and
            // released automatically when this __vm-boot process exits.
            // ponytail: flock is per-host; a cross-host lock manager only if
            // workers ever share storage across machines.
            use std::os::unix::io::AsRawFd;
            match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&upper)
            {
                Ok(lock) => {
                    // Bounded retry: on a restart, the old __vm-boot can be
                    // signalled but not yet reaped by the kernel, which has
                    // not yet dropped its leaked lock fd, so the first LOCK_NB here
                    // can lose the race against the dying VM. Retry for ~2s
                    // before giving up, so a normal restart doesn't surface as a
                    // spurious "already attached" that leaves the worker down. A
                    // genuine concurrent second start still fails after the
                    // window (the old VM never exits).
                    let mut waited_ms = 0u32;
                    let acquired = loop {
                        let rc =
                            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
                        if rc == 0 {
                            break true;
                        }
                        if waited_ms >= 2000 {
                            break false;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        waited_ms += 100;
                    };
                    if !acquired {
                        return Err(format!(
                            "overlay upper {upper} is still attached to a running VM after 2s; \
                             refusing to double-mount (concurrent start, or a prior VM that \
                             won't exit?)"
                        ));
                    }
                    std::mem::forget(lock);
                }
                Err(e) => return Err(format!("cannot lock overlay upper {upper}: {e}")),
            }
            eprintln!("iii: overlay writable upper -> /dev/vdb: {upper}");
            builder = builder.disk(move |d| {
                d.path(&upper)
                    .format(msb_krun::DiskImageFormat::Raw)
                    .read_only(false)
            });
        }
    }

    // 2 workers when the shell relay is active (benefits from
    // parallel read/write task scheduling); 1 otherwise (the network
    // runs fine on a single worker).
    let worker_threads = if args.shell_sock.is_some() { 2 } else { 1 };
    let tokio_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .enable_all()
        .build()
        .map_err(|e| format!("tokio runtime failed: {}", e))?;

    // Network plumbing is gated on `--network`. When disabled, no virtio-net
    // device is attached and the III_INIT_* env vars are left unset, so
    // iii-init's `configure_network()` short-circuits (network.rs:42-44).
    //
    // The triple is `Option<String>` so absence is explicit at the type level:
    // empty-string sentinels would silently turn `://localhost:` into `://:`
    // when the rewrite below ran with networking off (see `rewrite_localhost`).
    let (dns_nameserver, guest_ip, gateway_ip): (Option<String>, Option<String>, Option<String>) =
        if args.network {
            let net_cfg = iii_network::NetworkConfig {
                activity_file: args.net_activity_file.clone().map(std::path::PathBuf::from),
                ..Default::default()
            };
            let mut network = iii_network::SmoltcpNetwork::new(net_cfg, args.slot);
            network.start(tokio_rt.handle().clone());
            builder =
                builder.net(|net| net.mac(network.guest_mac()).custom(network.take_backend()));
            (
                Some(network.gateway_ipv4().to_string()),
                Some(network.guest_ipv4().to_string()),
                Some(network.gateway_ipv4().to_string()),
            )
        } else {
            (None, None, None)
        };

    // Skip the rewrite entirely when networking is off — no gateway exists,
    // and `localhost`/`127.0.0.1` resolve via the guest's loopback interface
    // unchanged.
    let rewrite_localhost =
        |s: &str| -> String { maybe_rewrite_localhost(s, gateway_ip.as_deref()) };
    let worker_cmd = rewrite_localhost(&worker_cmd);

    let worker_heap_mib = (args.ram as u64 * 3 / 4).max(128);
    let worker_heap_bytes = worker_heap_mib * 1024 * 1024;

    // Set up the shell-exec channel BEFORE constructing the exec
    // closure so `shell_port_env` reflects whether the host relay
    // actually started. Otherwise a failed `shell_relay::spawn` still
    // sets III_SHELL_PORT and the guest dispatcher spins on a port
    // nobody reads, silently breaking `sandbox::exec` for the VM's
    // lifetime. Fail-closed: on relay spawn failure, skip the env var
    // and drop the attached fd so the VM boots without the port.
    let mut shell_port_env = false;
    let mut guest_shell_fd: Option<i32> = None;
    let mut shell_sock_fingerprint: Option<crate::cli::shell_relay::ShellSocketFingerprint> = None;
    if let Some(sock_path) = args.shell_sock.clone() {
        let (host_end, guest_fd) = setup_port_socketpair()?;
        match crate::cli::shell_relay::spawn(
            tokio_rt.handle(),
            std::path::PathBuf::from(sock_path),
            host_end,
        ) {
            Ok(fp) => {
                shell_sock_fingerprint = Some(fp);
                guest_shell_fd = Some(guest_fd);
                shell_port_env = true;
            }
            Err(e) => {
                eprintln!(
                    "warning: shell relay failed to start ({e}). \
                     sandbox exec disabled; VM boots normally."
                );
                // `host_end` was moved into `shell_relay::spawn` and is
                // dropped on its Err path. Close our guest half so we
                // don't leak the fd for the __vm-boot process lifetime.
                unsafe {
                    libc::close(guest_fd);
                }
            }
        }
    }
    let mut source_sync_port_env = false;
    let mut guest_source_sync_fd: Option<i32> = None;
    if let Some(source_root) = args.source_sync.clone() {
        let (host_end, guest_fd) = setup_port_socketpair()?;
        match crate::cli::source_sync_host::spawn(std::path::PathBuf::from(source_root), host_end) {
            Ok(()) => {
                guest_source_sync_fd = Some(guest_fd);
                source_sync_port_env = true;
            }
            Err(error) => {
                eprintln!(
                    "warning: source sync failed to start ({error}). \
                     Host source changes will not reach the guest."
                );
                unsafe {
                    libc::close(guest_fd);
                }
            }
        }
    }
    // Worker env and command transport (MOT-5493). The kernel command line
    // is readable in the guest's `/proc/cmdline`, only carries printable
    // ASCII without quotes, and msb_krun panics on anything else. With an
    // embedded init, which is guaranteed to be this build's, both cross the
    // `iii.env` virtio-console port and iii-init exports them before
    // starting the worker. An on-disk init may predate the port, so that
    // case keeps the command line. Either way, whatever stays on the line is
    // validated below: a bad value is a clear error instead of a panic.
    let worker_env_pairs: Vec<(String, String)> = worker_env
        .map(|(key, value)| {
            let value = rewrite_localhost(&value);
            (key, value)
        })
        .collect();
    let use_env_port = iii_filesystem::init::has_init();
    let kv = |key: &str, value: &str| (key.to_string(), value.to_string());

    // iii-init's plumbing, in the order the old exec closure set it
    // (`split_vm_env` adds the worker command, the env and PATH).
    let mut plumbing: Vec<(String, String)> = Vec::new();
    // III_INIT_* are only meaningful when the smoltcp stack is wired up.
    // Leaving them unset makes iii-init's configure_network() a no-op.
    if let (Some(dns), Some(ip), Some(gw)) = (
        dns_nameserver.as_deref(),
        guest_ip.as_deref(),
        gateway_ip.as_deref(),
    ) {
        plumbing.push(kv("III_INIT_DNS", dns));
        plumbing.push(kv("III_INIT_IP", ip));
        plumbing.push(kv("III_INIT_GW", gw));
        plumbing.push(kv("III_INIT_CIDR", "30"));
    }
    plumbing.push(kv("III_WORKER_MEM_BYTES", &worker_heap_bytes.to_string()));
    // Overlay rootfs: tell iii-init to build `/` from the read-only base
    // disk (/dev/vda) + a writable upper instead of the virtiofs root.
    // The upper is a pre-formatted ext4 image on /dev/vdb (materialized
    // host-side from the embedded golden image) when one was attached;
    // iii-init mounts it as-is (no in-guest format). When absent (dev
    // builds without the embedded golden), fall back to a non-persistent
    // tmpfs upper so the overlay still boots — deps just don't persist.
    if overlay_mode {
        plumbing.push(kv("III_BLOCK_ROOT_LOWER", "/dev/vda"));
        plumbing.push(kv("III_BLOCK_ROOT_LOWER_FSTYPE", "erofs"));
        if args.rootfs_upper.is_some() {
            plumbing.push(kv("III_BLOCK_ROOT_UPPER", "/dev/vdb"));
            plumbing.push(kv("III_BLOCK_ROOT_UPPER_FSTYPE", "ext4"));
        } else {
            plumbing.push(kv("III_BLOCK_ROOT_UPPER", "tmpfs"));
        }
    }
    // iii-init starts the worker in this directory, in its own process group.
    plumbing.push(kv("III_WORKER_WORKDIR", &args.workdir));
    if shell_port_env {
        plumbing.push(kv(
            "III_SHELL_PORT",
            iii_supervisor::shell_protocol::SHELL_PORT_NAME,
        ));
    }
    if source_sync_port_env {
        plumbing.push(kv(
            "III_SOURCE_SYNC_PORT",
            iii_supervisor::source_sync::SOURCE_SYNC_PORT_NAME,
        ));
        plumbing.push(kv("III_SOURCE_SYNC_ROOT", "/workspace"));
        plumbing.push(kv("III_SOURCE_SYNC_READY", "/run/iii/source-ready"));
    }
    if !virtiofs_mount_env.is_empty() {
        plumbing.push(kv("III_VIRTIOFS_MOUNTS", &virtiofs_mount_env));
    }

    let VmEnvPlan {
        cmdline: cmdline_env,
        frame,
    } = split_vm_env(plumbing, worker_env_pairs, &worker_cmd, use_env_port);
    let mut guest_env_fd: Option<i32> = None;
    if let Some(frame_env) = frame {
        let frame = iii_supervisor::env_channel::encode_frame(&frame_env)
            .map_err(|e| format!("cannot pass the worker environment to the VM: {e}"))?;
        let (host_end, guest_fd) = setup_port_socketpair()?;
        spawn_env_writer(host_end, frame)?;
        guest_env_fd = Some(guest_fd);
    }

    // msb_krun also writes the workdir there, as KRUN_WORKDIR. It is not
    // quoted, so a space splits it into a stray token. That is harmless and
    // deliberately not rejected (it would refuse workdirs that boot today):
    // nothing in the guest reads KRUN_WORKDIR, and iii-init takes the
    // worker's directory from III_WORKER_WORKDIR, which is quoted.
    let mut on_cmdline = cmdline_env.clone();
    on_cmdline.push(kv("KRUN_WORKDIR", &args.workdir));
    validate_cmdline_env(&on_cmdline).map_err(|e| {
        if use_env_port {
            e
        } else {
            format!(
                "{e}. This init binary predates the iii.env channel; a sandbox-builtin \
                 built with an embedded init sends the worker env and command over it instead"
            )
        }
    })?;

    let nofile_limit = args.nofile_limit;

    builder = builder.exec(|mut e| {
        e = e.path("/init.krun").workdir(&args.workdir);
        // Bound the open-file table for the guest worker from the host
        // builder. iii-init also calls setrlimit(RLIMIT_NOFILE) as its
        // first act, but KRUN_RLIMITS is applied by init.krun before
        // iii-init's Rust code runs — belt-and-suspenders for fds opened
        // during dynamic loader setup and early allocator state.
        // nofile_limit=0 means "let the guest rlimit::raise_nofile path
        // own it" and skips the call.
        if nofile_limit > 0 {
            e = e.rlimit("RLIMIT_NOFILE", nofile_limit, nofile_limit);
        }
        for (key, value) in &cmdline_env {
            e = e.env(key, value);
        }
        e
    });

    // Configure the console device: each `port(name, fd, fd)` adds a named
    // virtio-console port backed by the guest-side end of a socketpair
    // (msb_krun builder.rs:367-391).
    // Shell-exec channel was already set up above (before the exec
    // closure was built) so `shell_port_env` correctly reflects whether
    // the relay actually spawned. `guest_shell_fd` is `Some` only when
    // spawn succeeded; on Err we dropped the guest fd to fail closed.

    builder = builder.console(move |mut c| {
        if let Some(fd) = guest_shell_fd {
            c = c.port(iii_supervisor::shell_protocol::SHELL_PORT_NAME, fd, fd);
        }
        if let Some(fd) = guest_source_sync_fd {
            c = c.port(iii_supervisor::source_sync::SOURCE_SYNC_PORT_NAME, fd, fd);
        }
        if let Some(fd) = guest_env_fd {
            c = c.port(iii_supervisor::env_channel::ENV_PORT_NAME, fd, fd);
        }
        c
    });

    // Register VM-exit cleanup for the shell socket.
    if let Some(fp) = shell_sock_fingerprint {
        builder = builder.on_exit(move |exit_code| {
            // Only unlink the shell socket if it's still the one we
            // bound — a fast `stop → start` can let a new VM rebind
            // the same path before this hook fires. The fingerprint
            // check makes stale unlinks a no-op.
            fp.remove_if_unchanged();
            if exit_code != 0 {
                eprintln!("  VM exited with code {}", exit_code);
            }
        });
    }

    let vm = builder
        .build()
        .map_err(|e| format!("VM build failed: {}", e))?;

    // Capture an exit handle BEFORE `enter()` moves `vm`. SIGTERM or
    // SIGINT delivered to this __vm-boot process now triggers the
    // VMM's clean shutdown path (event-loop reads the exit eventfd,
    // notifies on_exit observers, `_exit`s with the current exit
    // code) instead of the process being killed mid-guest-write.
    // Without this, `stop → SIGTERM` from the host leaves the guest
    // no chance to fsync or flush stdout logs; the on_exit closure
    // installed above unlinks the shell socket.
    //
    // Register the two signal streams SYNCHRONOUSLY before spawning
    // the awaiter task. Doing registration inside the spawned task
    // leaves a race window: SIGTERM arriving between `spawn` and the
    // task's first poll hits the default action (process kill).
    // Registering via `block_on` here closes that window — once the
    // block_on returns, the runtime's signal driver is already
    // listening; the spawned task just awaits the notification.
    let exit_handle_for_signals = vm.exit_handle();
    let signals_registered: Result<
        (tokio::signal::unix::Signal, tokio::signal::unix::Signal),
        std::io::Error,
    > = tokio_rt.block_on(async {
        use tokio::signal::unix::{SignalKind, signal};
        let sigterm = signal(SignalKind::terminate())?;
        let sigint = signal(SignalKind::interrupt())?;
        Ok((sigterm, sigint))
    });
    if let Ok((mut sigterm, mut sigint)) = signals_registered {
        tokio_rt.spawn(async move {
            tokio::select! {
                _ = sigterm.recv() => {}
                _ = sigint.recv() => {}
            }
            // `trigger` is async-signal-safe and idempotent per
            // msb_krun's ExitHandle docs — repeat fires after shutdown
            // has already begun are no-ops.
            exit_handle_for_signals.trigger();
        });
    } else {
        eprintln!(
            "  warning: failed to register SIGTERM/SIGINT handlers; \
             `stop` will fall back to abrupt SIGTERM kills."
        );
    }

    // Lifeline cascade: the adapter attaches a lifeline pipe to every VM it
    // boots, so tear the VM down the instant the adapter dies — ANY death,
    // SIGKILL included. Without this, VMs are session leaders that nothing
    // ever reaps once their adapter is gone. Routed through the same
    // ExitHandle as SIGTERM/SIGINT so the on_exit cleanup (shell socket
    // fingerprint) runs instead of an abrupt process::exit. Plain OS
    // threads, because libkrun owns the main thread once `enter()` runs; if
    // the spawner died during boot the first read/poll fires immediately.
    // The spawner-pid poll backstops the one lifeline failure mode (a write
    // end leaked through the macOS non-atomic CLOEXEC window delaying EOF).
    #[cfg(unix)]
    {
        if let Some(fd) = crate::daemon_exit::take_early_lifeline() {
            let handle = vm.exit_handle();
            std::thread::spawn(move || {
                if crate::daemon_exit::blocking_wait_lifeline_eof(fd) {
                    shut_down_vm(|| handle.trigger(), "spawner lifeline closed");
                }
            });
        }
        if let Some(pid) = crate::daemon_exit::early_spawner_pid() {
            let handle = vm.exit_handle();
            std::thread::spawn(move || {
                crate::daemon_exit::blocking_wait_pid_gone(pid);
                shut_down_vm(|| handle.trigger(), &format!("spawner pid {pid} exited"));
            });
        }
    }

    let vcpu_label = if args.vcpus == 1 { "vCPU" } else { "vCPUs" };
    eprintln!(
        "  Booting VM ({} {}, {} MiB RAM)...",
        args.vcpus, vcpu_label, args.ram
    );
    vm.enter().map_err(|e| format!("VM enter failed: {}", e))
}

/// The spawner is gone: shut the VM down, then say why. A worker
/// VM's stderr is a pipe the dead spawner read, so `eprintln!` would panic on
/// EPIPE and end this watcher before the trigger, leaving the VM running.
#[cfg(unix)]
fn shut_down_vm(trigger: impl FnOnce(), why: &str) {
    use std::io::Write;
    trigger();
    let _ = writeln!(std::io::stderr(), "vm-boot: {why}; shutting down VM");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vm_boot_args_parse() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            args: VmBootArgs,
        }

        let cli = TestCli::parse_from([
            "test",
            "--rootfs",
            "/tmp/rootfs",
            "--exec",
            "/usr/bin/python3",
            "--workdir",
            "/workspace",
            "--vcpus",
            "4",
            "--ram",
            "1024",
            "--env",
            "FOO=bar",
            "--arg",
            "script.py",
        ]);

        assert_eq!(cli.args.rootfs, "/tmp/rootfs");
        assert_eq!(cli.args.exec, "/usr/bin/python3");
        assert_eq!(cli.args.workdir, "/workspace");
        assert_eq!(cli.args.vcpus, 4);
        assert_eq!(cli.args.ram, 1024);
        assert_eq!(cli.args.env, vec!["FOO=bar"]);
        assert_eq!(cli.args.arg, vec!["script.py"]);
    }

    #[test]
    fn test_shell_quote_safe_chars() {
        assert_eq!(shell_quote("simple"), "simple");
        assert_eq!(shell_quote("/usr/bin/node"), "/usr/bin/node");
    }

    #[test]
    fn test_shell_quote_unsafe_chars() {
        assert_eq!(shell_quote("has space"), "'has space'");
    }

    #[test]
    fn test_build_worker_cmd_no_args() {
        assert_eq!(build_worker_cmd("/usr/bin/node", &[]), "/usr/bin/node");
    }

    #[test]
    fn test_build_worker_cmd_with_args() {
        let args = vec![
            "script.js".to_string(),
            "--port".to_string(),
            "3000".to_string(),
        ];
        assert_eq!(
            build_worker_cmd("/usr/bin/node", &args),
            "/usr/bin/node script.js --port 3000"
        );
    }

    #[test]
    fn build_virtiofs_mount_plan_empty() {
        let plan = build_virtiofs_mount_plan(&[]).unwrap();
        assert!(plan.entries.is_empty());
        assert!(plan.env_var.is_empty());
    }

    #[test]
    fn build_virtiofs_mount_plan_single() {
        let plan = build_virtiofs_mount_plan(&["/host/proj:/workspace".to_string()]).unwrap();
        assert_eq!(plan.entries.len(), 1);
        assert_eq!(plan.entries[0].tag, "virtiofs_0");
        assert_eq!(plan.entries[0].host_path, "/host/proj");
        assert_eq!(plan.entries[0].guest_path, "/workspace");
        assert_eq!(plan.env_var, "virtiofs_0=/workspace");
    }

    #[test]
    fn build_virtiofs_mount_plan_multiple_preserves_order_and_indexes_tags() {
        let plan = build_virtiofs_mount_plan(&["/host/a:/b".to_string(), "/host/c:/d".to_string()])
            .unwrap();
        assert_eq!(plan.entries.len(), 2);
        assert_eq!(plan.entries[0].tag, "virtiofs_0");
        assert_eq!(plan.entries[0].host_path, "/host/a");
        assert_eq!(plan.entries[0].guest_path, "/b");
        assert_eq!(plan.entries[1].tag, "virtiofs_1");
        assert_eq!(plan.entries[1].host_path, "/host/c");
        assert_eq!(plan.entries[1].guest_path, "/d");
        assert_eq!(plan.env_var, "virtiofs_0=/b;virtiofs_1=/d");
    }

    #[test]
    fn build_virtiofs_mount_plan_rejects_missing_colon() {
        let err = build_virtiofs_mount_plan(&["/host/noguest".to_string()]).unwrap_err();
        assert!(err.contains("Invalid mount format"));
    }

    #[test]
    fn build_virtiofs_mount_plan_rejects_empty_host() {
        let err = build_virtiofs_mount_plan(&[":/guest".to_string()]).unwrap_err();
        assert!(err.contains("Invalid mount format"));
    }

    #[test]
    fn build_virtiofs_mount_plan_rejects_empty_guest() {
        let err = build_virtiofs_mount_plan(&["/host:".to_string()]).unwrap_err();
        assert!(err.contains("Invalid mount format"));
    }

    // --- 6.1: check_kvm_nonexistent_path (Linux only) ---
    #[cfg(target_os = "linux")]
    #[test]
    fn test_check_kvm_nonexistent_path() {
        let result = check_kvm_at_path(std::path::Path::new("/dev/nonexistent_kvm"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    // --- 6.2: shell_quote with embedded single quotes ---
    #[test]
    fn test_shell_quote_with_embedded_single_quotes() {
        let result = shell_quote("it's a test");
        assert_eq!(result, "'it'\\''s a test'");
    }

    // --- 6.3: DEFAULT_GUEST_PATH (guards the npm shebang fallback added
    // when a rootfs cache lacks `.oci-config.json`). The fallback condition
    // itself is tested through `split_vm_env`. See the `DEFAULT_GUEST_PATH`
    // docstring for the full rationale.
    // --- MOT-5493: legacy kernel-cmdline transport must refuse, not panic.

    fn pair(key: &str, value: &str) -> Vec<(String, String)> {
        vec![(key.to_string(), value.to_string())]
    }

    #[test]
    fn cmdline_env_accepts_printable_ascii() {
        assert!(validate_cmdline_env(&pair("III_URL", "ws://10.0.2.2:49134")).is_ok());
        assert!(validate_cmdline_env(&pair("EMPTY", "")).is_ok());
        assert!(validate_cmdline_env(&pair("SPACES", "a b c")).is_ok());
    }

    #[test]
    fn cmdline_env_rejects_what_msb_krun_cannot_carry_naming_only_the_key() {
        for value in [
            "Ol\u{e1}",
            "\u{1f496}",
            "line1\nline2",
            "tab\there",
            r#"say "hi""#,
        ] {
            let err = validate_cmdline_env(&pair("API_KEY", value)).unwrap_err();
            assert!(err.contains("API_KEY"), "{err}");
            assert!(!err.contains(value), "value leaked into: {err}");
        }
    }

    #[test]
    fn cmdline_env_rejects_bad_names() {
        assert!(validate_cmdline_env(&pair("A B", "x")).is_err());
        assert!(validate_cmdline_env(&pair("CH\u{c3}VE", "x")).is_err());
    }

    #[test]
    fn cmdline_env_rejects_oversized_env() {
        let big = "x".repeat(CMDLINE_ENV_BUDGET);
        assert!(validate_cmdline_env(&pair("BIG", &big)).is_err());
    }

    #[test]
    fn env_writer_delivers_a_frame_larger_than_the_socket_buffer() {
        let (host, mut guest) = std::os::unix::net::UnixStream::pair().unwrap();
        // Over a Linux socketpair's ~180 KiB, so the writer really blocks;
        // each variable stays under the 128 KiB execve limit.
        let mut env: Vec<(String, String)> = (0..3)
            .map(|i| (format!("BIG{i}"), "x".repeat(100 * 1024)))
            .collect();
        env.push((
            "III_WORKER_CMD".to_string(),
            "exec bash /opt/iii/dev-run.sh".to_string(),
        ));
        let frame = iii_supervisor::env_channel::encode_frame(&env).unwrap();
        spawn_env_writer(host, frame).unwrap();
        assert_eq!(
            iii_supervisor::env_channel::read_frame(&mut guest).unwrap(),
            env
        );
        // Dropping `guest` ends the writer thread's keep-open loop.
    }

    #[test]
    fn workdir_and_plumbing_on_the_cmdline_are_checked_too() {
        let err = validate_cmdline_env(&[("KRUN_WORKDIR".to_string(), "/s\u{e3}o".to_string())])
            .unwrap_err();
        assert!(err.contains("KRUN_WORKDIR"), "{err}");
    }

    fn kvs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn port_path_keeps_worker_values_and_the_command_off_the_cmdline() {
        let plan = split_vm_env(
            kvs(&[("III_WORKER_MEM_BYTES", "1")]),
            kvs(&[
                ("API_KEY", "sk-secret"),
                ("III_WORKER_CMD", "echo HIJACKED"),
                ("III_ENV_PORT", "evil"),
            ]),
            "exec bash /opt/iii/dev-run.sh",
            true,
        );
        assert!(
            !plan
                .cmdline
                .iter()
                .any(|(k, v)| k == "API_KEY" || k == "III_WORKER_CMD" || v.contains("sk-secret")),
            "{:?}",
            plan.cmdline
        );
        assert!(
            plan.cmdline
                .contains(&("III_ENV_PORT".to_string(), "iii.env".to_string()))
        );
        let frame = plan.frame.expect("frame on the port path");
        assert_eq!(
            frame,
            kvs(&[
                ("API_KEY", "sk-secret"),
                ("III_WORKER_CMD", "exec bash /opt/iii/dev-run.sh"),
            ])
        );
    }

    #[test]
    fn legacy_path_keeps_the_old_order_and_drops_a_forwarded_command() {
        let plan = split_vm_env(
            kvs(&[("III_WORKER_MEM_BYTES", "1")]),
            kvs(&[("A", "1"), ("III_WORKER_CMD", "echo HIJACKED")]),
            "run",
            false,
        );
        assert!(plan.frame.is_none());
        assert_eq!(
            plan.cmdline,
            kvs(&[
                ("III_WORKER_CMD", "run"),
                ("III_WORKER_MEM_BYTES", "1"),
                ("A", "1"),
                ("PATH", DEFAULT_GUEST_PATH),
            ])
        );
    }

    #[test]
    fn path_fallback_only_when_the_worker_env_has_none() {
        let with = split_vm_env(vec![], kvs(&[("PATH", "/x")]), "run", true);
        assert!(!with.cmdline.iter().any(|(k, _)| k == "PATH"));
        assert!(
            with.frame
                .unwrap()
                .contains(&("PATH".to_string(), "/x".to_string()))
        );
        // Exact, case-sensitive name: `path` and `PATHEXT` are not PATH.
        let lookalikes = split_vm_env(
            vec![],
            kvs(&[("path", "/x"), ("PATHEXT", "y")]),
            "run",
            true,
        );
        assert!(
            lookalikes
                .cmdline
                .contains(&("PATH".to_string(), DEFAULT_GUEST_PATH.to_string()))
        );
        // An empty PATH is still the worker's own choice.
        let empty = split_vm_env(vec![], kvs(&[("PATH", "")]), "run", true);
        assert!(!empty.cmdline.iter().any(|(k, _)| k == "PATH"));
        let without = split_vm_env(vec![], vec![], "run", true);
        assert!(
            without
                .cmdline
                .contains(&("PATH".to_string(), DEFAULT_GUEST_PATH.to_string()))
        );
    }

    #[test]
    fn env_stdin_flag_parses() {
        #[derive(clap::Parser)]
        struct TestCli {
            #[command(flatten)]
            args: VmBootArgs,
        }
        use clap::Parser;
        let cli = TestCli::parse_from([
            "test",
            "--rootfs",
            "/tmp/rootfs",
            "--exec",
            "/bin/sh",
            "--env-stdin",
        ]);
        assert!(cli.args.env_stdin);
        assert!(cli.args.env.is_empty());
    }

    /// `__vm-boot --help` names no command of the retired `iii worker` CLI.
    #[test]
    fn help_names_no_iii_worker_command() {
        #[derive(clap::Parser)]
        struct TestCli {
            #[command(flatten)]
            args: VmBootArgs,
        }
        let help = <TestCli as clap::CommandFactory>::command()
            .render_long_help()
            .to_string();
        assert!(help.contains("--shell-sock"), "{help}");
        for retired in ["iii-worker", "iii worker"] {
            assert!(!help.contains(retired), "{retired}: {help}");
        }
    }

    #[test]
    fn default_guest_path_matches_debian_default() {
        // Smoke test: keep the constant aligned with `/etc/profile` on
        // the Debian-family base images the worker rootfs ships from.
        // If you change this, also bump the constant docstring and the
        // related test that asserts the fallback fix for shebang
        // scripts in /usr/local/bin.
        assert_eq!(
            DEFAULT_GUEST_PATH,
            "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
        );
    }

    /// A worker VM whose spawner (its stderr reader) died must still shut
    /// down. Re-runs itself as a child test process so fd 2 can be a broken
    /// pipe and `--nocapture` sends `eprintln!` there, as in `__vm-boot`.
    #[cfg(unix)]
    #[test]
    fn shut_down_vm_triggers_when_stderr_is_a_broken_pipe() {
        const CHILD: &str = "III_TEST_BROKEN_STDERR_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let mut fds = [0; 2];
            assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
            unsafe {
                libc::close(fds[0]);
                libc::dup2(fds[1], 2);
            }
            let (tx, rx) = std::sync::mpsc::channel();
            let watcher = std::thread::spawn(move || {
                shut_down_vm(move || tx.send(()).unwrap(), "spawner lifeline closed")
            });
            assert!(watcher.join().is_ok(), "the watcher panicked");
            assert!(rx.try_recv().is_ok(), "the VM was never triggered");
            return;
        }
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "cli::vm_boot::tests::shut_down_vm_triggers_when_stderr_is_a_broken_pipe",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&child.stdout);
        assert!(stdout.contains("1 passed"), "child test failed: {stdout}");
    }
}
