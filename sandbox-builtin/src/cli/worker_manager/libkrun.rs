// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! libkrun VM runtime for worker VMs and sandboxes.
//!
//! Provides VM-based isolated execution using libkrun (Apple Hypervisor.framework
//! on macOS, KVM on Linux). The VM runs in a separate helper process
//! for crash isolation.

use anyhow::Result;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Forward optional VM-boot tuning flags to the `__vm-boot` child
/// based on opt-in environment variables. Keeps the public API of
/// `build_vm_command` unchanged while giving
/// operators a single place to enable hyperthreading, nested
/// virtualization, virtiofs DAX window tuning, and the worker-side
/// NOFILE rlimit for perf experiments.
///
/// Variables read (all optional; omit to take each CLI default):
///   - `III_VM_HYPERTHREADING=1|true|on|yes` → `--hyperthreading`
///   - `III_VM_NESTED_VIRT=1|true|on|yes`   → `--nested-virt`
///   - `III_VM_VIRTIOFS_SHM_SIZE_MIB=<int>` → `--virtiofs-shm-size-mib <n>` (0 = skip)
///   - `III_VM_NOFILE_LIMIT=<int>`     → `--nofile-limit <n>` (0 = let iii-init own it)
///
/// Bad values are ignored silently (a typo in an opt-in perf flag
/// should never fail a worker boot). Appending args to both
/// `std::process::Command` and `tokio::process::Command` needs the
/// same logic, so we take `&mut CommandArgsExt` via the closure
/// rather than committing to either concrete type.
fn apply_vm_tuning_env(mut push: impl FnMut(&str, Option<&str>)) {
    let parse_bool =
        |v: &str| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "on" | "yes");

    if std::env::var("III_VM_HYPERTHREADING")
        .ok()
        .filter(|v| parse_bool(v))
        .is_some()
    {
        push("--hyperthreading", None);
    }
    if std::env::var("III_VM_NESTED_VIRT")
        .ok()
        .filter(|v| parse_bool(v))
        .is_some()
    {
        push("--nested-virt", None);
    }
    if let Some(n) = std::env::var("III_VM_VIRTIOFS_SHM_SIZE_MIB")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
    {
        let owned = n.to_string();
        push("--virtiofs-shm-size-mib", Some(&owned));
    }
    if let Some(n) = std::env::var("III_VM_NOFILE_LIMIT")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    {
        let owned = n.to_string();
        push("--nofile-limit", Some(&owned));
    }
}

/// msb_krun (the VMM) is compiled into the binary; this checks for libkrunfw.
pub fn libkrun_available() -> bool {
    crate::cli::firmware::resolve::resolve_libkrunfw_dir().is_some()
}

/// Build the VM boot env. Launcher wins: `III_ISOLATION=libkrun` is written
/// after caller env so an OCI image `ENV III_ISOLATION=docker` cannot override it.
pub(crate) fn build_vm_env(caller_env: HashMap<String, String>) -> HashMap<String, String> {
    let mut merged = HashMap::with_capacity(caller_env.len() + 1);
    for (key, value) in caller_env {
        merged.insert(key, value);
    }
    merged.insert("III_ISOLATION".to_string(), "libkrun".to_string());
    merged
}

/// A `__vm-boot` invocation, built but not spawned.
///
/// The caller spawns it and keeps the handle, to stop the VM and to see it
/// crash.
pub struct VmCommand {
    pub command: tokio::process::Command,
    /// The worker env, as the `__vm-boot --env-stdin` frame. The command
    /// carries no env values in argv, where `ps` would show them to every
    /// local user (MOT-5492): whoever spawns it must write this to the
    /// child's stdin and close it (see [`feed_env_stdin`]).
    pub env_stdin: Vec<u8>,
}

/// Writes [`VmCommand::env_stdin`] into a spawned `__vm-boot` and closes the
/// pipe. `__vm-boot` reads the frame before anything else, so this returns
/// as soon as the frame is in the pipe.
pub async fn feed_env_stdin(
    stdin: Option<tokio::process::ChildStdin>,
    frame: &[u8],
) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;

    let mut stdin =
        stdin.ok_or_else(|| std::io::Error::other("the VM boot process has no stdin pipe"))?;
    stdin.write_all(frame).await?;
    stdin.shutdown().await
}

/// The binary that answers `__vm-boot`: always the running one, so the VMM,
/// init and firmware come from one build. Searching `PATH` would let a stray
/// older copy boot the VM.
fn vm_boot_program() -> Result<PathBuf, String> {
    // Test-only: an integration test that drives `WorkerVms` in process runs
    // in a test binary, which cannot answer `__vm-boot`.
    #[cfg(feature = "integration-vm")]
    if let Some(bin) = std::env::var_os("III_TEST_VM_BOOT_BIN") {
        return Ok(bin.into());
    }
    std::env::current_exe().map_err(|e| format!("cannot locate the running binary: {e}"))
}

/// Builds the `__vm-boot` command for one VM. No process is started.
#[allow(clippy::too_many_arguments)]
pub fn build_vm_command(
    exec_path: &str,
    args: &[String],
    env: HashMap<String, String>,
    vcpus: u32,
    ram_mib: u32,
    rootfs: PathBuf,
    rootfs_lower: Option<PathBuf>,
    rootfs_mode: String,
    rootfs_upper: Option<PathBuf>,
    mounts: &[(String, String)],
    source_sync: Option<&Path>,
) -> Result<VmCommand, String> {
    let env = build_vm_env(env);

    let self_exe = vm_boot_program()?;

    #[cfg(target_os = "macos")]
    {
        if let Err(e) = super::platform::ensure_macos_entitlements(&self_exe) {
            eprintln!(
                "warning: failed to codesign for Hypervisor entitlement: {}",
                e
            );
        }
    }

    let env_pairs: Vec<(String, String)> = env.into_iter().collect();
    let env_stdin = iii_supervisor::env_channel::encode_frame(&env_pairs)
        .map_err(|e| format!("cannot pass the worker environment to the VM: {e}"))?;
    let shell_sock = rootfs.join("shell.sock");

    let mut cmd = tokio::process::Command::new(&self_exe);
    cmd.arg("__vm-boot");
    for boot_arg in vm_boot_args_dev(
        &rootfs,
        rootfs_lower.as_deref(),
        &rootfs_mode,
        rootfs_upper.as_deref(),
        exec_path,
        vcpus,
        ram_mib,
        &shell_sock,
        source_sync,
        mounts,
        args,
    ) {
        cmd.arg(boot_arg);
    }

    apply_vm_tuning_env(|flag, val| {
        cmd.arg(flag);
        if let Some(v) = val {
            cmd.arg(v);
        }
    });

    if let Some(fw_dir) = crate::cli::firmware::resolve::resolve_libkrunfw_dir() {
        cmd.env(
            crate::cli::firmware::resolve::lib_path_env_var(),
            fw_dir.to_string_lossy().as_ref(),
        );
    }

    cmd.stdin(std::process::Stdio::piped());

    // Deliberately no `setsid` here: how the child is detached belongs to
    // whoever spawns it. A caller that supervises its own children puts them in
    // a process group it can signal, and `setsid` fails once the child is
    // already a group leader, and the spawn fails with it.
    Ok(VmCommand {
        command: cmd,
        env_stdin,
    })
}

const VM_BOOT_NETWORK_FLAG: &str = "--network";
/// The env goes on stdin, never argv, where `ps` shows it (MOT-5492).
const VM_BOOT_ENV_STDIN_FLAG: &str = "--env-stdin";

/// Push `--rootfs-lower <sqfs> --rootfs-mode <mode>` when overlay mode is
/// active (i.e. the start path built a base erofs). No-op for legacy.
fn push_overlay_args(
    out: &mut Vec<OsString>,
    rootfs_lower: Option<&Path>,
    rootfs_mode: &str,
    rootfs_upper: Option<&Path>,
) {
    if let Some(lower) = rootfs_lower {
        out.push(OsString::from("--rootfs-lower"));
        out.push(lower.as_os_str().to_owned());
        out.push(OsString::from("--rootfs-mode"));
        out.push(OsString::from(rootfs_mode));
        // The upper is optional even in overlay mode: dev builds without an
        // embedded golden image attach no upper and iii-init falls back to a
        // tmpfs upper.
        if let Some(upper) = rootfs_upper {
            out.push(OsString::from("--rootfs-upper"));
            out.push(upper.as_os_str().to_owned());
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn vm_boot_args_dev(
    rootfs: &Path,
    rootfs_lower: Option<&Path>,
    rootfs_mode: &str,
    rootfs_upper: Option<&Path>,
    exec_path: &str,
    vcpus: u32,
    ram_mib: u32,
    shell_sock: &Path,
    source_sync: Option<&Path>,
    mounts: &[(String, String)],
    exec_args: &[String],
) -> Vec<OsString> {
    let mut out: Vec<OsString> = Vec::new();
    out.push(OsString::from("--rootfs"));
    out.push(rootfs.as_os_str().to_owned());
    push_overlay_args(&mut out, rootfs_lower, rootfs_mode, rootfs_upper);
    out.push(OsString::from("--exec"));
    out.push(OsString::from(exec_path));
    out.push(OsString::from("--workdir"));
    out.push(OsString::from("/workspace"));
    out.push(OsString::from("--vcpus"));
    out.push(OsString::from(vcpus.to_string()));
    out.push(OsString::from("--ram"));
    out.push(OsString::from(ram_mib.to_string()));
    out.push(OsString::from("--shell-sock"));
    out.push(shell_sock.as_os_str().to_owned());
    if let Some(source_sync) = source_sync {
        out.push(OsString::from("--source-sync"));
        out.push(source_sync.as_os_str().to_owned());
    }
    out.push(OsString::from(VM_BOOT_NETWORK_FLAG));
    out.push(OsString::from(VM_BOOT_ENV_STDIN_FLAG));
    for (host, guest) in mounts {
        out.push(OsString::from("--mount"));
        out.push(OsString::from(format!("{}:{}", host, guest)));
    }
    for arg in exec_args {
        out.push(OsString::from("--arg"));
        out.push(OsString::from(arg.as_str()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vm_boot_program_is_the_running_binary() {
        assert_eq!(vm_boot_program().unwrap(), std::env::current_exe().unwrap());
    }

    fn parse_vm_boot_args(boot_args: Vec<OsString>) -> crate::cli::vm_boot::VmBootArgs {
        use clap::Parser;

        #[derive(Parser)]
        struct Wrapper {
            #[command(flatten)]
            args: crate::cli::vm_boot::VmBootArgs,
        }

        let mut argv: Vec<OsString> = vec![OsString::from("test")];
        argv.extend(boot_args);
        Wrapper::parse_from(argv).args
    }

    #[test]
    fn vm_boot_args_dev_enables_network_and_roundtrips() {
        let mounts = vec![("/host/src".to_string(), "/guest/src".to_string())];
        let exec_args = vec!["-c".to_string(), "exec bash /run.sh".to_string()];

        let parsed = parse_vm_boot_args(vm_boot_args_dev(
            Path::new("/tmp/rootfs"),
            None,
            "",
            None,
            "/bin/sh",
            2,
            2048,
            Path::new("/tmp/shell.sock"),
            Some(Path::new("/host/src")),
            &mounts,
            &exec_args,
        ));

        assert!(parsed.network);
        assert_eq!(parsed.rootfs, "/tmp/rootfs");
        assert_eq!(parsed.exec, "/bin/sh");
        assert_eq!(parsed.workdir, "/workspace");
        assert_eq!(parsed.vcpus, 2);
        assert_eq!(parsed.ram, 2048);
        assert_eq!(parsed.shell_sock.as_deref(), Some("/tmp/shell.sock"));
        assert_eq!(parsed.source_sync.as_deref(), Some("/host/src"));
        assert!(parsed.env.is_empty());
        assert!(parsed.env_stdin);
        assert_eq!(parsed.mount, vec!["/host/src:/guest/src".to_string()]);
        assert_eq!(parsed.arg, exec_args);
    }

    #[tokio::test]
    async fn feed_env_stdin_writes_the_frame_and_closes_the_pipe() {
        let mut child = tokio::process::Command::new("cat")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let env = vec![("API_KEY".to_string(), "sk-secret".to_string())];
        let frame = iii_supervisor::env_channel::encode_frame(&env).unwrap();
        feed_env_stdin(child.stdin.take(), &frame).await.unwrap();
        // `cat` only exits once it sees EOF, so this also proves the close.
        let out = child.wait_with_output().await.unwrap();
        assert_eq!(
            iii_supervisor::env_channel::read_frame(&mut out.stdout.as_slice()).unwrap(),
            env
        );
    }

    #[tokio::test]
    async fn feed_env_stdin_without_a_pipe_is_an_error() {
        assert!(feed_env_stdin(None, b"").await.is_err());
    }

    #[test]
    fn test_libkrun_available_returns_bool() {
        let result = libkrun_available();
        let _ = result;
    }

    #[test]
    fn build_vm_env_injects_isolation_marker_into_empty_input() {
        let merged = build_vm_env(HashMap::new());
        assert_eq!(merged.get("III_ISOLATION"), Some(&"libkrun".to_string()));
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn build_vm_env_preserves_caller_vars_and_adds_isolation() {
        let mut caller = HashMap::new();
        caller.insert("NODE_ENV".to_string(), "production".to_string());
        caller.insert("III_URL".to_string(), "ws://127.0.0.1:3111".to_string());
        let merged = build_vm_env(caller);
        assert_eq!(merged.get("III_ISOLATION"), Some(&"libkrun".to_string()));
        assert_eq!(merged.get("NODE_ENV"), Some(&"production".to_string()));
        assert_eq!(
            merged.get("III_URL"),
            Some(&"ws://127.0.0.1:3111".to_string())
        );
        assert_eq!(merged.len(), 3);
    }

    #[test]
    fn build_vm_env_launcher_overrides_caller_isolation() {
        let mut caller = HashMap::new();
        caller.insert("III_ISOLATION".to_string(), "docker".to_string());
        let merged = build_vm_env(caller);
        assert_eq!(merged.get("III_ISOLATION"), Some(&"libkrun".to_string()));
    }
}
