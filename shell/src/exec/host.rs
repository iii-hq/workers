//! Host-process exec backend. Wraps `tokio::process::Command` with the
//! existing env-scrubbing, bounded-output, and timeout-watchdog logic
//! that lived in the flat `src/exec.rs` before the trait split.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::config::ShellConfig;
use crate::exec::policy::ExecOverrides;
use crate::exec::{backend::ExecBackend, error::ExecError, ExecCallResult, ExecOutcome};

pub fn parse_argv(command: &str, args: Option<&Vec<String>>) -> Result<Vec<String>, String> {
    if let Some(args) = args {
        let mut v = vec![command.to_string()];
        v.extend(args.iter().cloned());
        Ok(v)
    } else {
        shell_words::split(command).map_err(|e| format!("parse command: {}", e))
    }
}

/// Build the host `Command`. `overrides` carries the already-validated per-call
/// `cwd`/`env` (see `crate::exec::policy`): when both are `None` (the common
/// case) this is byte-for-byte the prior behaviour. Per-call values are applied
/// AFTER the config-derived defaults so an override wins for that one key /
/// directory; the argv itself is unchanged (the denylist already ran
/// in the handler).
pub fn build_command(
    argv: &[String],
    cfg: &ShellConfig,
    overrides: &ExecOverrides,
) -> Result<Command, String> {
    let program = argv.first().ok_or_else(|| "empty command".to_string())?;
    let mut cmd = Command::new(program);
    if argv.len() > 1 {
        cmd.args(&argv[1..]);
    }
    if !cfg.env.inherit {
        cmd.env_clear();
        for k in &cfg.env.allow {
            if let Ok(v) = std::env::var(k) {
                cmd.env(k, v);
            }
        }
    }
    // Per-call env overrides are applied LAST so a permitted key's per-call
    // value wins over the config-forwarded value. Keys were already gated
    // against env.allow + DANGEROUS_ENV_KEYS in the handler, so this loop
    // trusts the validated map. Note: when env.inherit is true the child
    // already inherits the worker's full env; the override still sets these
    // keys explicitly on top.
    if let Some(env) = &overrides.env {
        for (k, v) in env {
            cmd.env(k, v);
        }
    }
    // Per-call cwd (already jail-confined to a canonical, existing dir) wins
    // over cfg.working_dir; absent, fall back to the config default (prior
    // behaviour).
    if let Some(dir) = &overrides.cwd {
        cmd.current_dir(dir);
    } else if let Some(dir) = &cfg.working_dir {
        cmd.current_dir(dir);
    }
    // stdin is piped when the caller supplied bytes (the spawn site writes them
    // then closes the pipe so the child sees EOF); otherwise /dev/null, the
    // prior default, so a command that reads stdin doesn't block forever.
    cmd.stdin(if overrides.stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Tokio's Command does NOT kill children on drop by default. Both the
        // foreground exec path and the background spawn build via this helper;
        // on shutdown the runtime drops the detached tasks owning each Child,
        // so without this they would become orphaned processes outliving the
        // worker. This is the backstop; main() also runs an explicit kill sweep.
        .kill_on_drop(true);
    // Put the child in its own process group (it becomes the group leader, so
    // pgid == pid). Termination then signals the WHOLE group via `-pid`, killing
    // any grandchildren the command spawned — not just the direct child, which
    // `start_kill`/`kill_on_drop` alone would leave orphaned. The group kill is
    // always issued while the caller still owns the un-reaped Child, so the pgid
    // cannot have been reused.
    #[cfg(unix)]
    cmd.process_group(0);
    Ok(cmd)
}

/// SIGKILL an entire process group by its leader pid. The caller MUST still own
/// the un-reaped `Child` for `pid` when calling this, so the pid (and therefore
/// the pgid, since children are spawned with `process_group(0)`) cannot have been
/// reused by the OS — this is what makes signalling by number safe. The negated
/// pid targets the whole group, terminating grandchildren too. No-op on non-unix;
/// a vanished process (ESRCH) is ignored.
#[cfg(unix)]
pub(crate) fn kill_process_group(pid: Option<u32>) {
    if let Some(pid) = pid {
        // SAFETY: see the doc above — `pid` is an un-reaped child we own, and the
        // negative value signals its process group.
        unsafe {
            libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
pub(crate) fn kill_process_group(_pid: Option<u32>) {}

/// Feed `stdin` (if any) to the child's stdin pipe, then close it so the child
/// sees EOF. Runs in a detached task so a child that interleaves reading stdin
/// with writing stdout cannot deadlock against our stdout/stderr drains
/// (each side would otherwise block on a full pipe). Best-effort: a child that
/// exited before consuming its input just makes the write fail, which we ignore.
pub(crate) fn pump_stdin(child: &mut tokio::process::Child, stdin: &Option<String>) {
    if let Some(data) = stdin {
        if let Some(mut sin) = child.stdin.take() {
            let bytes = data.clone().into_bytes();
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                let _ = sin.write_all(&bytes).await;
                let _ = sin.shutdown().await;
            });
        }
    }
}

pub async fn run_to_completion(
    argv: &[String],
    cfg: &ShellConfig,
    timeout_ms: u64,
    overrides: &ExecOverrides,
) -> Result<ExecOutcome, String> {
    let started = std::time::Instant::now();
    let mut cmd = build_command(argv, cfg, overrides)?;
    let mut child = cmd.spawn().map_err(|e| format!("spawn: {}", e))?;
    pump_stdin(&mut child, &overrides.stdin);

    let mut stdout_reader = child.stdout.take().ok_or("no stdout pipe")?;
    let mut stderr_reader = child.stderr.take().ok_or("no stderr pipe")?;

    let limit = cfg.max_output_bytes;
    let timeout = Duration::from_millis(timeout_ms);

    let stdout_task = tokio::spawn(async move { read_bounded(&mut stdout_reader, limit).await });
    let stderr_task = tokio::spawn(async move { read_bounded(&mut stderr_reader, limit).await });

    let wait_res = tokio::time::timeout(timeout, child.wait()).await;

    let (exit_code, timed_out) = match wait_res {
        Ok(Ok(status)) => (status.code(), false),
        Ok(Err(e)) => return Err(format!("wait: {}", e)),
        Err(_) => {
            // Hard timeout: kill the whole process group (grandchildren too) while
            // we still own the un-reaped child, then reap. start_kill is the
            // non-unix fallback (kill_process_group is a no-op there).
            kill_process_group(child.id());
            let _ = child.start_kill();
            let _ = child.wait().await;
            (None, true)
        }
    };

    let (stdout_bytes, stdout_truncated) =
        stdout_task.await.map_err(|e| format!("stdout: {}", e))?;
    let (stderr_bytes, stderr_truncated) =
        stderr_task.await.map_err(|e| format!("stderr: {}", e))?;

    Ok(ExecOutcome {
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        exit_code,
        duration_ms: started.elapsed().as_millis() as u64,
        timed_out,
        stdout_truncated,
        stderr_truncated,
    })
}

async fn read_bounded<R: AsyncReadExt + Unpin>(reader: &mut R, limit: usize) -> (Vec<u8>, bool) {
    let mut buf = Vec::with_capacity(limit.min(8192));
    let mut chunk = [0u8; 8192];
    let mut truncated = false;
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                if !truncated {
                    if buf.len() + n > limit {
                        let take = limit.saturating_sub(buf.len());
                        buf.extend_from_slice(&chunk[..take]);
                        truncated = true;
                        // Fall through and continue draining; do NOT break.
                        // Dropping the reader here closes the pipe, and the
                        // child's next write() raises SIGPIPE — making
                        // ExitStatus::code() report None on what should have
                        // been a normal-exit-with-truncated-output run.
                    } else {
                        buf.extend_from_slice(&chunk[..n]);
                    }
                }
                // truncated == true: discard everything past `limit`.
            }
            Err(_) => break,
        }
    }
    (buf, truncated)
}

pub struct HostExecBackend {
    cfg: Arc<ShellConfig>,
}

impl HostExecBackend {
    pub fn new(cfg: Arc<ShellConfig>) -> Self {
        Self { cfg }
    }
}

#[async_trait]
impl ExecBackend for HostExecBackend {
    async fn run(
        &self,
        argv: &[String],
        timeout_ms: u64,
        overrides: &ExecOverrides,
    ) -> ExecCallResult<ExecOutcome> {
        // S216 is the generic "shell-internal error" code in the
        // S2xx range. We deliberately do NOT use S300 here even
        // though it visually parallels SandboxExecBackend's S300
        // path: in iii's S-code taxonomy, S300 is reserved for "VM
        // boot failed" (no virt host). Wrapping host-side spawn or
        // wait failures as S300 would make a "command not found" on
        // the host indistinguishable from a missing /dev/kvm to
        // callers that branch on the code.
        run_to_completion(argv, &self.cfg, timeout_ms, overrides)
            .await
            .map_err(|e| ExecError::new("S216", format!("host exec: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cfg() -> ShellConfig {
        let mut c = ShellConfig {
            env: crate::config::EnvConfig::inherit_all(),
            max_output_bytes: 4096,
            ..Default::default()
        };
        c.compile_denylist().unwrap();
        c
    }

    #[test]
    fn test_parse_argv_with_args_field() {
        let got = parse_argv("echo", Some(&vec!["hello".into(), "world".into()])).unwrap();
        assert_eq!(got, vec!["echo", "hello", "world"]);
    }

    #[test]
    fn test_parse_argv_from_shell_words() {
        let got = parse_argv(r#"echo "hello world""#, None).unwrap();
        assert_eq!(got, vec!["echo", "hello world"]);
    }

    #[test]
    fn test_parse_argv_bad_quoting() {
        assert!(parse_argv(r#"echo "unterminated"#, None).is_err());
    }

    #[tokio::test]
    async fn test_run_echo() {
        let cfg = test_cfg();
        let out = run_to_completion(
            &["echo".into(), "hi".into()],
            &cfg,
            5000,
            &ExecOverrides::default(),
        )
        .await
        .unwrap();
        assert_eq!(out.exit_code, Some(0));
        assert_eq!(out.stdout.trim(), "hi");
        assert!(!out.timed_out);
    }

    #[tokio::test]
    async fn test_run_nonexistent_command() {
        let cfg = test_cfg();
        let err = run_to_completion(
            &["_nope_no_exist_".into()],
            &cfg,
            1000,
            &ExecOverrides::default(),
        )
        .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_timeout_kills() {
        let cfg = test_cfg();
        let out = run_to_completion(
            &["sleep".into(), "5".into()],
            &cfg,
            200,
            &ExecOverrides::default(),
        )
        .await
        .unwrap();
        assert!(out.timed_out);
        assert_eq!(out.exit_code, None);
    }

    #[tokio::test]
    async fn test_output_truncation() {
        let mut cfg = test_cfg();
        cfg.max_output_bytes = 16;
        let out = run_to_completion(
            &[
                "sh".into(),
                "-c".into(),
                "printf 'x%.0s' $(seq 1 100)".into(),
            ],
            &cfg,
            3000,
            &ExecOverrides::default(),
        )
        .await
        .unwrap();
        assert!(out.stdout_truncated);
        assert_eq!(out.stdout.len(), 16);
    }

    #[tokio::test]
    async fn host_backend_runs_via_trait() {
        let cfg = Arc::new(test_cfg());
        let backend = HostExecBackend::new(cfg);
        let out = backend
            .run(
                &["echo".into(), "via-trait".into()],
                5000,
                &ExecOverrides::default(),
            )
            .await
            .unwrap();
        assert_eq!(out.exit_code, Some(0));
        assert_eq!(out.stdout.trim(), "via-trait");
    }

    /// A jail-confined `cwd` actually changes the child's working directory:
    /// `pwd` prints the override, not the worker's cwd or `cfg.working_dir`.
    /// Engine-free host path, reusing the existing harness.
    #[tokio::test]
    async fn cwd_override_runs_command_in_that_directory() {
        let root = std::env::temp_dir().join(format!("shell-cwd-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("workdir")).unwrap();
        let mut cfg = test_cfg();
        cfg.fs.host_roots = vec![root.clone()];

        let overrides =
            crate::exec::policy::build_overrides(Some("workdir"), None, None, None, &cfg)
                .expect("workdir is inside the jail");
        let out = run_to_completion(&["pwd".into()], &cfg, 5000, &overrides)
            .await
            .unwrap();
        let expected = root.join("workdir").canonicalize().unwrap();
        assert_eq!(out.stdout.trim(), expected.to_string_lossy());
        assert_eq!(out.exit_code, Some(0));
        std::fs::remove_dir_all(&root).ok();
    }

    /// A per-call `scope_root` with NO explicit `cwd` makes the session directory
    /// the child's working directory: `pwd` prints scope_root, not the worker's
    /// cwd or `cfg.working_dir`. Proves exec both confines to AND cwds at
    /// scope_root.
    #[tokio::test]
    async fn scope_root_becomes_child_working_directory() {
        let root = std::env::temp_dir().join(format!("shell-cwd-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("session")).unwrap();
        let mut cfg = test_cfg();
        cfg.fs.host_roots = vec![root.clone()];
        let base = root.join("session").to_string_lossy().into_owned();

        let overrides = crate::exec::policy::build_overrides(None, None, Some(&base), None, &cfg)
            .expect("session is inside the jail");
        let out = run_to_completion(&["pwd".into()], &cfg, 5000, &overrides)
            .await
            .unwrap();
        let expected = root.join("session").canonicalize().unwrap();
        assert_eq!(out.stdout.trim(), expected.to_string_lossy());
        assert_eq!(out.exit_code, Some(0));
        std::fs::remove_dir_all(&root).ok();
    }

    /// Removes the named process env vars on drop, even if the test body
    /// panics between `set_var` and where a plain cleanup call would have
    /// run — an assertion failure without this leaks the var into every
    /// later test in the binary. Unique per-test var names (see the caller)
    /// keep the residual risk to a logical leak, not a data race: full
    /// concurrent-mutation safety would need a process-wide env mutex shared
    /// with `code/config.rs`'s `CODER_TEST_ROOT` test, a larger change than
    /// this test warrants on its own.
    static ENV_TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Holds [`ENV_TEST_MUTEX`] for its whole lifetime AND
    /// removes the named process env vars on drop, even if the test body
    /// panics between `set_var` and where a plain cleanup call would have
    /// run. The mutex serializes against every other test in the crate that
    /// touches process env (see that constant's doc comment for why); the
    /// Drop cleanup handles the panic case the mutex alone doesn't cover.
    /// Held across `.await` below — fine under the default `#[tokio::test]`
    /// current-thread flavor, where the outer test future need not be
    /// `Send`.
    struct EnvVarGuard {
        keys: &'static [&'static str],
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl EnvVarGuard {
        fn new(keys: &'static [&'static str]) -> Self {
            let lock = ENV_TEST_MUTEX
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            Self { keys, _lock: lock }
        }
    }
    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            for k in self.keys {
                std::env::remove_var(k);
            }
        }
    }

    /// With `env.inherit: false`, the child env is scrubbed to exactly the
    /// `env.allow` keys: an allowed worker var round-trips, a non-allowed one
    /// never reaches the child. (Unit twin of the e2e scrub/passthrough cases.)
    #[tokio::test]
    async fn inherit_false_forwards_only_allow_keys() {
        const ALLOWED: &str = "SHELL_DX_ALLOWED_9F3A";
        const BLOCKED: &str = "SHELL_DX_BLOCKED_9F3A";
        // Guard acquired BEFORE set_var: it holds the cross-test env mutex,
        // so the sets below can't race with another test's env mutation.
        let _guard = EnvVarGuard::new(&[ALLOWED, BLOCKED]);
        std::env::set_var(ALLOWED, "allowed-value");
        std::env::set_var(BLOCKED, "blocked-value");

        let mut cfg = test_cfg();
        cfg.env.inherit = false;
        cfg.env.allow = vec!["PATH".into(), ALLOWED.into()];

        let out = run_to_completion(
            &["env".into()],
            &cfg,
            5000,
            &crate::exec::policy::ExecOverrides::default(),
        )
        .await
        .unwrap();
        assert!(
            out.stdout.contains("SHELL_DX_ALLOWED_9F3A=allowed-value"),
            "allowed key forwarded: {}",
            out.stdout
        );
        assert!(
            !out.stdout.contains("SHELL_DX_BLOCKED_9F3A"),
            "non-allowed key scrubbed: {}",
            out.stdout
        );
    }

    /// A permitted `env` key is visible to the child process. We forward
    /// `printenv NODE_ENV`; with NODE_ENV in env.allow and a per-call value,
    /// the child sees it.
    #[tokio::test]
    async fn env_override_is_visible_to_child() {
        let mut cfg = test_cfg();
        cfg.env.allow = vec!["NODE_ENV".into()];

        let mut env = std::collections::BTreeMap::new();
        env.insert("NODE_ENV".to_string(), "from-override".to_string());
        let overrides = crate::exec::policy::build_overrides(None, Some(&env), None, None, &cfg)
            .expect("NODE_ENV is allowlisted");

        let out = run_to_completion(
            &["printenv".into(), "NODE_ENV".into()],
            &cfg,
            5000,
            &overrides,
        )
        .await
        .unwrap();
        assert_eq!(out.stdout.trim(), "from-override");
    }

    /// An env key NOT in env.allow is rejected (S210) before any spawn,
    /// naming the offending key — the call never reaches the child.
    #[tokio::test]
    async fn env_key_outside_allow_list_is_rejected_s210() {
        let mut cfg = test_cfg();
        cfg.env.allow = vec!["NODE_ENV".into()];
        let mut env = std::collections::BTreeMap::new();
        env.insert("SECRET_TOKEN".to_string(), "x".to_string());
        let err = crate::exec::policy::build_overrides(None, Some(&env), None, None, &cfg)
            .expect_err("non-allowlisted key must reject");
        assert_eq!(err.code, "S210");
        assert!(
            err.message.contains("SECRET_TOKEN"),
            "names key: {}",
            err.message
        );
    }

    /// LD_PRELOAD is rejected (S210) even when the test also adds it to
    /// env.allow — proof that the dangerous-key denylist wins over the
    /// operator's allowlist.
    #[tokio::test]
    async fn dangerous_env_key_rejected_even_if_allowlisted_on_host_path() {
        let mut cfg = test_cfg();
        cfg.env.allow = vec!["LD_PRELOAD".into(), "NODE_ENV".into()];
        let mut env = std::collections::BTreeMap::new();
        env.insert("LD_PRELOAD".to_string(), "/tmp/evil.so".to_string());
        let err = crate::exec::policy::build_overrides(None, Some(&env), None, None, &cfg)
            .expect_err("LD_PRELOAD must reject even when allowlisted");
        assert_eq!(err.code, "S210");
        assert!(err.message.contains("LD_PRELOAD"));
    }

    /// A `cwd` resolving outside the jail is rejected S215 on the host path.
    #[tokio::test]
    async fn cwd_escaping_jail_rejected_s215_on_host_path() {
        let root = std::env::temp_dir().join(format!("shell-cwd-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mut cfg = test_cfg();
        cfg.fs.host_roots = vec![root.clone()];
        let err = crate::exec::policy::build_overrides(Some("../../etc"), None, None, None, &cfg)
            .expect_err("escape must reject");
        assert_eq!(err.code, "S215");
        std::fs::remove_dir_all(&root).ok();
    }

    /// Omitting both cwd and env preserves prior behaviour: the empty override
    /// runs the command exactly as before (no cwd change, no extra env).
    #[tokio::test]
    async fn empty_overrides_preserve_default_behaviour() {
        let cfg = test_cfg();
        let out = run_to_completion(
            &["echo".into(), "default".into()],
            &cfg,
            5000,
            &ExecOverrides::default(),
        )
        .await
        .unwrap();
        assert_eq!(out.stdout.trim(), "default");
        assert_eq!(out.exit_code, Some(0));
    }
}
