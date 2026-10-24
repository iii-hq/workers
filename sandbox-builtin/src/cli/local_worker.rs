// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Local-path and bundle worker VMs: `local_vm_command` and
//! `bundle_vm_command` build the `__vm-boot` command for a worker directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::project::{ProjectInfo, WORKER_MANIFEST, load_from_manifest};
use super::rootfs::clone_rootfs;

const PREPARED_MARKER_VERSION: &str = "workspace-copy-in-v1";

// ──────────────────────────────────────────────────────────────────────────────
// Shared helpers (extracted from dev.rs)
// ──────────────────────────────────────────────────────────────────────────────

fn prepared_marker_matches_current_workspace(marker: &Path) -> bool {
    std::fs::read_to_string(marker).is_ok_and(|contents| contents.trim() == PREPARED_MARKER_VERSION)
}

/// Write `dev-run.sh` (mode 0755) into the share and return its dir:
/// `runtime/` (mounted at guest /opt/iii) under overlay, `opt/iii` in the
/// legacy clone. Both are guest-writable, so neither the dir chain nor the
/// script is followed through a symlink the guest planted there.
fn write_run_script(managed_dir: &Path, overlay: bool, script: &str) -> std::io::Result<PathBuf> {
    use super::worker_manager::oci;
    let rel = if overlay { "runtime" } else { "opt/iii" };
    let dir = oci::ensure_rootfs_dir_no_follow(managed_dir, rel)?;
    oci::write_rootfs_file_no_follow(&dir.join("dev-run.sh"), script.as_bytes(), Some(0o755))?;
    Ok(dir)
}

/// Best-effort read of `resources.cpus`/`resources.memory` for VM sizing.
/// Lenient by design: any read/parse/shape failure falls back to the defaults
/// (2 vCPUs, 2048 MiB) — strictness lives in the add/start validation and
/// `worker::validate`, not in this sizing probe. Backed by the typed
/// [`super::worker_manifest::WorkerManifest`] so add, start, and validate all
/// read resources through one schema (previously this used the `serde_yml`
/// fork while everything else used `serde_yaml`).
pub fn parse_manifest_resources(manifest_path: &Path) -> (u32, u32) {
    let default = (2, 2048);
    let Ok(Some(doc)) = super::project::read_manifest_doc(manifest_path) else {
        return default;
    };
    let Ok(manifest) = super::worker_manifest::WorkerManifest::from_value(&doc) else {
        return default;
    };
    let r = manifest.resources.unwrap_or_default();
    (r.cpus.unwrap_or(2), r.memory.unwrap_or(2048))
}

/// Build the boot script that `iii-init` will exec as the worker command.
///
/// No supervisor wrapping: the in-VM `iii-init` binary absorbs the
/// supervisor role (see `iii_init::supervisor`) and opens the control
/// channel itself when the host sets `III_CONTROL_PORT`. That removes
/// the separate `/opt/iii/supervisor` binary and its install plumbing
/// that this function used to emit as `exec /opt/iii/supervisor ...`.
///
/// `is_bundle` controls workspace behavior:
///   * local-path workers copy host source into a guest-local `/workspace`;
///   * bundle workers mount their immutable install directory at `/workspace`.
pub fn build_libkrun_local_script(
    project: &ProjectInfo,
    prepared: bool,
    is_bundle: bool,
    overlay: bool,
) -> String {
    let env_exports = build_env_exports(&project.env);
    let mut parts = vec![
        "set -e".to_string(),
        "export HOME=${HOME:-/root}".to_string(),
        "export PATH=/usr/local/bin:/usr/bin:/bin:$PATH".to_string(),
        "export LANG=${LANG:-C.UTF-8}".to_string(),
    ];

    // Workspace strategy:
    //
    //  - BUNDLE: the immutable install dir is mounted live at /workspace via
    //    virtiofs; vendored deps ship inside it, so no dep handling is needed.
    //
    //  - LOCAL: the host project is mounted temporarily at /mnt/host-src and
    //    copied into /workspace. The share is detached before user code runs.
    //    Later changes arrive over `iii.source-sync` and are written by
    //    iii-init, which makes the guest kernel emit native inotify events.
    //    Dependency and build directories remain VM-local and are excluded
    //    from both the initial copy and later synchronization.
    //
    // The mountpoint checks guard against iii-init's mount_virtiofs_shares()
    // swallowing a mount failure as a warning (leaving the path
    // existing-but-unmounted), which would otherwise silently work against the
    // wrong backing store.
    if is_bundle {
        parts.push(
            r#"if ! { mountpoint -q /workspace 2>/dev/null || awk '$5 == "/workspace" && / - virtiofs /' /proc/self/mountinfo | grep -q .; }; then
  echo "iii: ERROR /workspace is not a virtiofs mountpoint (share missing or mount failed)" >&2
  echo "--- III_VIRTIOFS_MOUNTS=${III_VIRTIOFS_MOUNTS:-<unset>} ---" >&2
  cat /proc/self/mountinfo >&2 2>/dev/null || cat /proc/mounts >&2 2>/dev/null || mount >&2
  exit 1
fi
cd /workspace
echo "iii: workspace ready (bundle worker; dep-dir bind-mounts skipped — vendored deps are shipped in the bundle)" >&2"#
                .to_string(),
        );
    } else {
        parts.push(
            r#"SRC=/mnt/host-src
mkdir -p /workspace
if ! { mountpoint -q "$SRC" 2>/dev/null || awk -v s="$SRC" '$5 == s && / - virtiofs /' /proc/self/mountinfo | grep -q .; }; then
  echo "iii: ERROR $SRC (host source share) is not a virtiofs mountpoint (share missing or mount failed)" >&2
  echo "--- III_VIRTIOFS_MOUNTS=${III_VIRTIOFS_MOUNTS:-<unset>} ---" >&2
  cat /proc/self/mountinfo >&2 2>/dev/null || cat /proc/mounts >&2 2>/dev/null || mount >&2
  exit 1
fi
if ! command -v tar >/dev/null 2>&1; then
  echo "iii: ERROR workspace copy-in requires 'tar' in the base image" >&2
  exit 1
fi
# Re-sync deletions: /workspace lives on the persistent ext4 upper and tar -x
# only adds/overwrites (never deletes), so a source file removed on the host
# would keep running across a watcher restart. Clear every top-level entry
# EXCEPT the VM-local dep/build dirs (excluded from the copy, must persist),
# then re-extract the current source. On first boot /workspace is empty so this
# is a no-op. (Exclusion list MUST match the tar --exclude list below.)
find /workspace -mindepth 1 -maxdepth 1 \
  ! -name node_modules ! -name .venv ! -name target ! -name dist \
  ! -name __pycache__ ! -name .pytest_cache ! -name .next ! -name .git \
  -exec rm -rf {} +
# Copy host source into the VM-local /workspace, excluding dep/build dirs (at
# any depth) so VM-local installs persist across a re-copy and host clutter
# isn't pulled in. Only /mnt/host-src is read; the host project is never
# written, so no empty dep folders appear there.
( set -o pipefail; ( cd "$SRC" && tar -cf - --exclude=node_modules --exclude=.venv --exclude=target --exclude=dist --exclude=__pycache__ --exclude=.pytest_cache --exclude=.next --exclude=.git . ) | ( cd /workspace && tar -xpf - ) ) || { echo "iii: ERROR workspace copy-in failed" >&2; exit 1; }
# Enforce "host project untouched": detach the initial source share before
# user code runs. Later edits arrive as file operations over iii.source-sync.
cd /
umount "$SRC" || { echo "iii: ERROR could not detach $SRC; refusing to run with host source writable" >&2; exit 1; }
mkdir -p /run/iii
touch /run/iii/source-ready
cd /workspace
echo "iii: workspace ready (copy-in from host; deps stay VM-local, host project untouched)" >&2"#
                .to_string(),
        );
    }

    parts.push("echo $$ > /sys/fs/cgroup/worker/cgroup.procs 2>/dev/null || true".to_string());

    // Host source changes are transported into /workspace by iii-init. The
    // resulting guest-side filesystem operations produce normal inotify
    // events; the command declared by the worker owns restart behavior.

    // Provisioning (setup + install) gating:
    //   - legacy: the host validates the versioned marker in the per-worker
    //     clone and omits the block once the current workspace layout is ready.
    //   - overlay: the marker lives on the persistent ext4 upper, which the
    //     host CANNOT see (it's inside a block device). So always emit the
    //     block but guard it IN-GUEST on the marker. The upper persists across
    //     restarts, so install runs once and is skipped thereafter — matching
    //     legacy's skip semantics without needing host visibility. `set -e`
    //     still aborts (and leaves the marker unwritten) if install fails.
    if overlay {
        let mut prov = String::new();
        if !project.setup_cmd.is_empty() {
            prov.push_str(&project.setup_cmd);
            prov.push('\n');
        }
        if !project.install_cmd.is_empty() {
            prov.push_str(&project.install_cmd);
            prov.push('\n');
        }
        // `sync` flushes the just-installed deps + the marker from the guest
        // page cache to the ext4 upper (/dev/vdb) after install finishes.
        // Without it, a worker that exits within ext4's ~5s commit window tears
        // the VM down before the write lands, losing both and re-running install
        // next boot. ponytail: belt-and-suspenders with iii-init's sync-on-exit
        // (supervisor::sync_and_exit); this also covers a SIGKILL that lands
        // after install but before the worker exits.
        parts.push(format!(
            r#"if [ "$(cat /var/.iii-prepared 2>/dev/null || true)" != "{PREPARED_MARKER_VERSION}" ]; then
rm -f /var/.iii-prepared
{prov}mkdir -p /var
printf '%s\n' '{PREPARED_MARKER_VERSION}' > /var/.iii-prepared
sync
fi"#
        ));
    } else if !prepared {
        if !project.setup_cmd.is_empty() {
            parts.push(project.setup_cmd.clone());
        }
        if !project.install_cmd.is_empty() {
            parts.push(project.install_cmd.clone());
        }
        parts.push(format!(
            r#"mkdir -p /var && printf '%s\n' '{PREPARED_MARKER_VERSION}' > /var/.iii-prepared"#
        ));
    }

    // Exec the user command directly. When the host has configured a
    // control port (III_CONTROL_PORT env set by vm_boot), `iii-init`
    // enters supervisor mode and wraps this command internally with
    // restart/shutdown RPC handling. When no control port is set,
    // iii-init takes its legacy path and just supervises the one
    // process until it exits.
    // `exec` binds tighter than `&&`/`;`: a compound run_cmd like
    // "pip install -e . && python -m src.main" would parse as
    // `exec pip install -e .` — the shell is replaced by the FIRST
    // command, the rest never runs, and the VM silently tears down when
    // it exits. Re-quote the whole command into a single `sh -c` word so
    // exec applies to the entire scripts.start string; sh tail-execs
    // simple commands anyway, so single-command starts keep their PID
    // semantics.
    let quoted_run_cmd = format!("'{}'", project.run_cmd.replace('\'', "'\\''"));
    parts.push(format!(
        "{} && exec /bin/sh -c {}",
        env_exports, quoted_run_cmd
    ));
    parts.join("\n")
}

pub fn build_env_exports(env: &HashMap<String, String>) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (k, v) in env {
        if k == "III_ENGINE_URL" || k == "III_URL" || k == "III_WORKER_NAME" {
            continue;
        }
        if !k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') || k.is_empty() {
            continue;
        }
        parts.push(format!("export {}='{}'", k, shell_escape(v)));
    }
    if parts.is_empty() {
        "true".to_string()
    } else {
        parts.join(" && ")
    }
}

pub fn shell_escape(s: &str) -> String {
    s.replace('\'', "'\\''")
}

pub fn build_local_env(
    engine_url: &str,
    worker_name: &str,
    project_env: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("III_ENGINE_URL".to_string(), engine_url.to_string());
    env.insert("III_URL".to_string(), engine_url.to_string());
    // The config.yaml entry name, so SDKs can self-report the managed
    // identity. Engine truth (`iii worker status`/`list`, MOT-3931) matches
    // connections by this name; a worker that reports the SDK's
    // `hostname:pid` fallback instead reads as "not registered" forever.
    env.insert("III_WORKER_NAME".to_string(), worker_name.to_string());
    for (key, value) in project_env {
        if key != "III_ENGINE_URL" && key != "III_URL" && key != "III_WORKER_NAME" {
            env.insert(key.clone(), value.clone());
        }
    }
    env
}

/// Build the virtiofs mount list for a local-path worker: the host project
/// dir is shared at `workspace_guest`. Returns `(host_path, guest_path)` pairs
/// suitable for `libkrun::build_vm_command`.
///
/// `workspace_guest` is `/workspace` for the live-mount models (bundle, legacy)
/// and `/mnt/host-src` for the overlay W1 copy-in model (the script copies it
/// into a VM-local `/workspace`, so the host project is read-only in practice
/// and never gets the empty dep-dir folders).
pub fn build_local_mounts(project_path: &Path, workspace_guest: &str) -> Vec<(String, String)> {
    vec![(
        project_path.to_string_lossy().into_owned(),
        workspace_guest.to_string(),
    )]
}

/// Return `true` when `managed_dir` needs a rootfs clone before VM boot.
///
/// The predicate is `!managed_dir.join("bin").exists()` — every OCI base
/// image we ship populates `/bin`, and none of the failure-mode artifacts
/// (iii-init runtime mkdirs for /dev /proc /sys /etc /tmp /run, our own
/// side-effect writes under /opt /workspace) do. Plain
/// `!managed_dir.exists()` is insufficient: a half-start that created
/// runtime mountpoints then died would falsely register as "already
/// prepared" and surface as a confusing `supervisor spawn_initial failed:
/// No such file or directory` from iii-init once the VM tried to exec
/// `/bin/sh`. Extracted as a named helper so the regression tests can
/// exercise the exact predicate the start path uses.
fn needs_rootfs_clone(managed_dir: &std::path::Path) -> bool {
    !managed_dir.join("bin").exists()
}

/// Builds the VM for a local-path worker without starting it.
///
/// Compose owns the child lifecycle, so this returns the boot command and uses
/// caller-scoped VM state. Local manifest rules still apply: setup and install
/// are allowed, resources are not bundle-clamped, and the manifest name does
/// not have to match the compose container key.
pub async fn local_vm_command(
    worker_name: &str,
    worker_dir: &Path,
    run_override: Option<&str>,
    over: VmOverride<'_>,
) -> Result<super::worker_manager::libkrun::VmCommand, String> {
    start_worker_impl(
        worker_name,
        &worker_dir.to_string_lossy(),
        /*package=*/ None,
        over,
        run_override,
    )
    .await
}

/// A local VM runs only on an explicit, valid OCI image: an invalid VM request
/// must not silently change into another image.
fn validate_compose_base_image(worker_dir: &Path) -> Result<(), String> {
    let manifest_path = worker_dir.join(WORKER_MANIFEST);
    let doc = super::project::read_manifest_doc(&manifest_path)?
        .ok_or_else(|| format!("{} does not exist", manifest_path.display()))?;
    let value = doc
        .get("runtime")
        .and_then(|runtime| runtime.get("base_image"))
        .ok_or_else(|| {
            format!(
                "{} must declare `runtime.base_image` to run as a compose VM",
                manifest_path.display()
            )
        })?;
    let image = value.as_str().map(str::trim).ok_or_else(|| {
        format!(
            "{}: `runtime.base_image` must be a string",
            manifest_path.display()
        )
    })?;
    if !super::project::is_plausible_image_ref(image) {
        return Err(format!(
            "{}: `runtime.base_image` must be a plausible OCI image reference",
            manifest_path.display()
        ));
    }
    Ok(())
}

/// Builds the VM for an installed bundle worker without starting it.
///
/// For a caller that supervises its own children — compose stops them, watches
/// them exit, and captures their output — and therefore needs the handle rather
/// than a detached VM tracked by a pid file. Every bundle gate applies: the
/// operator kill switch, the manifest re-validation that assumes the install
/// dir may have been tampered with since install, and the start-time resource
/// clamp.
///
/// The returned command is not spawned, so the caller decides how its output is
/// captured and when it dies.
///
/// `package` is the name the bundle's manifest must declare; `worker_name` is
/// the container key, which may differ.
pub async fn bundle_vm_command(
    package: &str,
    worker_name: &str,
    install_dir: &Path,
    over: VmOverride<'_>,
) -> Result<super::worker_manager::libkrun::VmCommand, String> {
    start_worker_impl(
        worker_name,
        &install_dir.to_string_lossy(),
        Some(package),
        over,
        None,
    )
    .await
}

/// What a caller supplies when it spawns and supervises the VM itself.
///
/// The worker name is not unique across callers (two projects may use the
/// same container key), so the VM state and the engine URL come from the
/// caller.
pub struct VmOverride<'a> {
    /// Per-caller VM state: rootfs (or overlay trampoline), boot script, pid
    /// file.
    pub state_dir: PathBuf,
    /// Engine URL the guest connects to.
    pub engine_url: &'a str,
    /// Merged over the manifest's own env, and under the reserved keys
    /// [`build_local_env`] owns.
    pub extra_env: HashMap<String, String>,
}

/// The checks a worker source must pass before anything is pulled or booted:
/// the bundle kill switch, the manifest gates, project detection and, for a
/// local worker, the compose base image. Returns the project and its kind.
///
/// `package` is `Some` for a bundle, and names what its manifest must declare.
pub(crate) fn validate_source(
    worker_dir: &Path,
    package: Option<&str>,
    run_override: Option<&str>,
) -> Result<(ProjectInfo, String), String> {
    let project_path = worker_dir;
    if let Some(package) = package
        && super::bundle_download::bundle_workers_disabled()
    {
        return Err(format!(
            "bundle workers are disabled via {}=1; refusing to start '{package}'",
            super::bundle_download::ENV_BUNDLE_WORKERS_DISABLED,
        ));
    }

    // 1. Validate worker_path directory exists
    if !project_path.is_dir() {
        return Err(format!(
            "Worker path '{}' does not exist or is not a directory",
            project_path.display()
        ));
    }

    // 1a. Bundle workers: re-run the strict manifest validator before
    // we touch the publisher-controlled YAML via load_from_manifest. The
    // install-time validator (handle_bundle_add) already ran, but the
    // install dir is on the host filesystem — any local-FS-write
    // attacker (or a future bug that lets a bundle write to its own
    // install dir) could modify ~/.iii/workers-bundle/{name}/iii.worker.yaml
    // between install and start to introduce scripts.setup,
    // scripts.install, or runtime.base_image. The permissive
    // load_from_manifest path below would honor those fields. The
    // strict validator also enforces the 64 KiB manifest cap, which
    // defuses billion-laughs YAML expansion before serde_yaml sees it.
    if let Some(package) = package
        && let Err(e) = super::bundle_download::validate_bundle_manifest(project_path, package)
    {
        return Err(format!(
            "bundle manifest re-validation failed at start: {}",
            e,
        ));
    }

    // 1b. Local (non-bundle) workers: the manifest must exist, and gets
    // strict key validation, so a manifest hand-edited after it was written
    // can't smuggle unknown keys (or an oversize/billion-laughs YAML) past
    // the engine via the permissive load_from_manifest path below.
    let manifest_path = project_path.join(WORKER_MANIFEST);
    if package.is_none() {
        let doc = super::project::read_manifest_doc(&manifest_path)?
            .ok_or_else(|| format!("{} does not exist", manifest_path.display()))?;
        if let Err(e) = super::project::validate_manifest_keys(&doc, &manifest_path) {
            return Err(format!("manifest validation failed at start: {}", e));
        }
    }

    // 2. Load the manifest
    let mut project = load_from_manifest(&manifest_path).ok_or_else(|| {
        format!(
            "Could not load the worker manifest in '{}'",
            project_path.display()
        )
    })?;

    if let Some(run_override) = run_override {
        project.run_cmd = run_override.to_string();
    }

    if let Err(msg) = project.validate() {
        return Err(msg.to_string());
    }

    // Treat an empty/whitespace `kind:` field in the manifest the same
    // as missing — otherwise a YAML typo like `kind: ""` would silently
    // pass an empty string through the oci/inferred-script lookups and
    // fall into their `_` defaults, giving the wrong base image instead
    // of the documented "typescript" default.
    let kind = project
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .unwrap_or("typescript")
        .to_string();

    if package.is_none() {
        validate_compose_base_image(project_path)?;
    }
    Ok((project, kind))
}

/// Shared body for `local_vm_command` and `bundle_vm_command`: validates the
/// source, prepares the rootfs and boot script, and builds the `__vm-boot`
/// command without spawning it.
///
/// `package` is `Some` for a bundle, which changes resource and workspace
/// handling: resources are parsed and clamped via
/// `bundle_download::parse_bundle_resources` (saturating + capped) rather than
/// the permissive `parse_manifest_resources`, and the install dir is mounted
/// live instead of copied in.
pub(crate) async fn start_worker_impl(
    worker_name: &str,
    worker_path: &str,
    package: Option<&str>,
    over: VmOverride<'_>,
    run_override: Option<&str>,
) -> Result<super::worker_manager::libkrun::VmCommand, String> {
    let project_path = Path::new(worker_path);
    let (project, kind) = validate_source(project_path, package, run_override)?;
    let kind = kind.as_str();
    let is_bundle = package.is_some();

    // 3. Ensure libkrunfw available
    if let Err(e) = super::firmware::provision::ensure_libkrunfw().await {
        tracing::warn!(error = %e, "failed to ensure libkrunfw");
    }

    if !super::worker_manager::libkrun::libkrun_available() {
        return Err("No runtime available.\n  \
             Rebuild with --features embed-libkrunfw or place libkrunfw in ~/.iii/lib/"
            .to_string());
    }

    // 4. Prepare managed dir — clone rootfs on first start
    let managed_dir = over.state_dir;

    // Detect "managed_dir exists but the rootfs was never cloned here"
    // by probing for `/bin` — every OCI base image we ship has it, and
    // neither iii-init's runtime mount dirs (/dev, /proc, /sys, /etc,
    // /tmp, /run) nor our own side-effect writes (/opt, /workspace)
    // populate it.
    //
    // Matches the `path.join("bin").exists()` idiom in
    // `worker_manager/oci.rs::prepare_rootfs`. A plain `managed_dir.exists()`
    // check is too weak: any prior half-start (VM booted, iii-init mkdir'd
    // /dev/proc/sys, then died before finishing) leaves the dir present
    // but FHS-less, so the next start would skip the clone and boot a VM
    // with no `/bin/sh` — which surfaces as a confusing
    // "supervisor spawn_initial failed: No such file or directory" from
    // iii-init rather than a clear "rootfs never got populated" error.
    //
    // Self-heal: if the marker is missing but the dir exists, wipe it
    // first so `cp -c -a SRC DEST` (rootfs::clone_rootfs) creates DEST
    // fresh as a copy of SRC. Without the wipe, cp's "copy INTO existing
    // dir" semantics would nest the rootfs at `managed_dir/python/`
    // instead of at `managed_dir/`. Anything the user cares about under
    // managed_dir is regenerated on boot (dep caches come back after
    // pip install / npm install).
    // Overlay mode serves the shared read-only erofs base as the rootfs
    // lower (built later from the cache) plus a per-worker ext4 upper — so
    // there is NO per-worker rootfs clone. The managed_dir is just a minimal
    // trampoline: embed-init injects /init.krun from memory (overlay requires
    // an embedded init) and iii-init creates its runtime mount dirs at boot.
    // This is what eliminates both the per-worker clone AND the empty dep
    // dirs the legacy clone left behind.
    let overlay = crate::cli::overlay::overlay_active();
    if overlay {
        if let Err(e) = std::fs::create_dir_all(&managed_dir) {
            return Err(format!(
                "Failed to create worker dir {}: {}",
                managed_dir.display(),
                e
            ));
        }
    } else if needs_rootfs_clone(&managed_dir) {
        eprintln!("  Preparing sandbox...");
        if managed_dir.exists()
            && let Err(e) = std::fs::remove_dir_all(&managed_dir)
        {
            return Err(format!(
                "Failed to clear stale managed dir {}: {}",
                managed_dir.display(),
                e
            ));
        }
        let base_rootfs =
            match super::worker_manager::oci::prepare_rootfs(kind, project.base_image.as_deref())
                .await
            {
                Ok(p) => p,
                Err(e) => {
                    return Err(e.to_string());
                }
            };
        if let Err(e) = clone_rootfs(&base_rootfs, &managed_dir) {
            return Err(format!("Failed to create project rootfs: {}", e));
        }
    }

    // 5. Workspace staging. Bundle installs are shared live at `/workspace`.
    //    Local workers use `/mnt/host-src` only for initial copy-in, in both
    //    legacy and overlay modes, so the host repo is never written.
    //    Pre-create the managed-dir mountpoint so the legacy clone (which IS
    //    the guest root) has /workspace to mount onto; harmless under overlay.
    if let Err(e) =
        super::worker_manager::oci::ensure_rootfs_dir_no_follow(&managed_dir, "workspace")
    {
        return Err(format!(
            "Failed to create workspace dir {}: {}",
            managed_dir.join("workspace").display(),
            e
        ));
    }

    let is_prepared =
        prepared_marker_matches_current_workspace(&managed_dir.join("var").join(".iii-prepared"));

    // 6. Build env with engine URL + OCI env + the caller's env
    let mut combined_project_env = project.env.clone();
    combined_project_env.extend(over.extra_env);

    let mut env = build_local_env(over.engine_url, worker_name, &combined_project_env);

    let base_rootfs =
        match super::worker_manager::oci::prepare_rootfs(kind, project.base_image.as_deref()).await
        {
            Ok(p) => p,
            Err(e) => {
                return Err(e.to_string());
            }
        };
    let oci_env = super::worker_manager::oci::read_oci_env(&base_rootfs);
    for (key, value) in oci_env {
        env.entry(key).or_insert(value);
    }

    // 7. Build boot script. No supervisor wrap is emitted — the in-VM
    //    `iii-init` binary absorbs that role (see iii_init::supervisor).
    //    Fast-restart is enabled whenever vm_boot wires the control port,
    //    which sets `III_CONTROL_PORT` in the guest env for iii-init.
    let script = build_libkrun_local_script(&project, is_prepared, is_bundle, overlay);

    // Both modes exec `bash /opt/iii/dev-run.sh`; only the SOURCE of that file
    // differs:
    //   - legacy: written into the per-worker clone at managed_dir/opt/iii,
    //     which IS the guest root (PassthroughFs) — directly visible.
    //   - overlay: after pivot the guest root is the read-only erofs lower
    //     + ext4 upper, which the host can't write into directly. Write the
    //     script to a host runtime dir and virtiofs-mount it at /opt/iii
    //     (iii-init mounts virtiofs shares POST-pivot, so it lands in the
    //     overlay root).
    //
    // The script is delivered as a FILE, never inlined into III_WORKER_CMD:
    // with an on-disk (not embedded) init the guest env rides the kernel
    // cmdline (krun_env), whose size `vm_boot` caps. Keeping III_WORKER_CMD
    // tiny (`exec bash /opt/iii/dev-run.sh`) keeps the env within that budget
    // for both modes.
    let script_dir = write_run_script(&managed_dir, overlay, &script)
        .map_err(|e| format!("Failed to write run script: {}", e))?;
    // The script sets up the workspace; no pre-cd needed (and /workspace is
    // empty until virtiofs mounts, so cd-before-exec would be racy).
    let exec_path = "/bin/sh";
    let args = vec![
        "-c".to_string(),
        "exec bash /opt/iii/dev-run.sh".to_string(),
    ];

    // 8. Copy iii-init if needed
    let init_path = match super::firmware::provision::ensure_init_binary().await {
        Ok(p) => p,
        Err(e) => {
            return Err(format!("Failed to provision iii-init: {}", e));
        }
    };

    if !iii_filesystem::init::has_init() {
        // The share is guest-writable, so init.krun may be a planted symlink: never copy through it.
        let dest = managed_dir.join("init.krun");
        if let Err(e) = std::fs::read(&init_path).and_then(|bytes| {
            super::worker_manager::oci::write_rootfs_file_no_follow(&dest, &bytes, Some(0o755))
        }) {
            return Err(format!("Failed to copy iii-init to rootfs: {}", e));
        }
    }

    // 9. Run via libkrun
    let manifest_path = project_path.join(WORKER_MANIFEST);
    let (vcpus, ram) = if is_bundle {
        // Strict clamp at start-time so a publisher-controlled manifest
        // can't boot a libkrun guest with attacker-chosen resources.
        // Install-time clamp (handle_bundle_add) only emits a WARN
        // event; the actual VM boot still re-reads the manifest from
        // the install dir, which is exactly what a malicious bundle
        // counts on.
        match super::bundle_download::parse_bundle_resources(
            project_path,
            super::bundle_download::ResourceCaps::default(),
        ) {
            Ok(r) => {
                if let Some(req) = r.clamped_cpus {
                    tracing::warn!(
                        worker = %worker_name,
                        bundle.requested_cpus = req,
                        bundle.cpus = r.cpus,
                        "bundle resources.cpus clamped at start"
                    );
                }
                if let Some(req) = r.clamped_memory_mb {
                    tracing::warn!(
                        worker = %worker_name,
                        bundle.requested_memory_mb = req,
                        bundle.memory_mb = r.memory_mb,
                        "bundle resources.memory clamped at start"
                    );
                }
                (r.cpus, r.memory_mb)
            }
            Err(e) => {
                // Manifest was validated at install time. If reading
                // it fails now, something tampered with the install
                // dir or the disk is failing. Fall back to safe
                // defaults rather than booting with raw values.
                tracing::error!(
                    worker = %worker_name,
                    error = %e,
                    "bundle resource re-clamp failed; using safe defaults"
                );
                (2, 2048)
            }
        }
    } else {
        parse_manifest_resources(&manifest_path)
    };

    // Local workers always copy into /workspace. This is required for native
    // guest file events: later host changes are applied by iii-init rather than
    // appearing as out-of-band mutations on a live virtiofs mount.
    let copy_in = !is_bundle;
    let workspace_guest = if copy_in {
        "/mnt/host-src"
    } else {
        "/workspace"
    };
    let mut mounts = build_local_mounts(project_path, workspace_guest);
    if overlay {
        // Make the host-written dev-run.sh visible in the (post-pivot) overlay
        // root at /opt/iii. iii-init mkdir_p's the guest path before mounting,
        // and overlayfs makes /opt writable (upper) even though the erofs
        // lower is read-only.
        mounts.push((
            script_dir.to_string_lossy().into_owned(),
            "/opt/iii".to_string(),
        ));
    }

    // Overlay: build the shared read-only base erofs from the prepared base
    // rootfs (host-side, image-independent) and materialize this worker's
    // persistent ext4 upper from the embedded golden image, both when overlay
    // is active. Built in the start path so the cost is visible and a failure
    // fails the start cleanly (rather than inside the detached __vm-boot child),
    // and never silently falls back to a per-worker clone.
    let (rootfs_lower, rootfs_mode, rootfs_upper) = if overlay {
        let erofs_img = match crate::cli::erofs::ensure_base_erofs(&base_rootfs) {
            Ok(s) => s,
            Err(e) => {
                return Err(format!("failed to build base erofs: {}", e));
            }
        };
        let upper = if crate::cli::upper::has_golden() {
            match crate::cli::upper::ensure_upper_ext4(&managed_dir) {
                Ok(p) => Some(p),
                Err(e) => {
                    return Err(format!("failed to materialize overlay upper: {}", e));
                }
            }
        } else {
            None
        };
        (Some(erofs_img), "overlay".to_string(), upper)
    } else {
        (None, String::new(), None)
    };

    // The caller spawns, waits on and stops the VM.
    super::worker_manager::libkrun::build_vm_command(
        exec_path,
        &args,
        env,
        vcpus,
        ram,
        managed_dir,
        rootfs_lower,
        rootfs_mode,
        rootfs_upper,
        &mounts,
        (!is_bundle).then_some(project_path),
    )
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// A guest that ran `ln -sf ~/.bashrc /opt/iii/dev-run.sh` (or symlinked
    /// `opt` itself) must not get the host file overwritten on the next start.
    #[test]
    fn write_run_script_never_writes_through_a_guest_symlink() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let t = tempfile::tempdir().unwrap();
        let victim = t.path().join("bashrc");
        std::fs::write(&victim, "host").unwrap();
        for overlay in [true, false] {
            let share = t.path().join(format!("share-{overlay}"));
            let dir = share.join(if overlay { "runtime" } else { "opt/iii" });
            std::fs::create_dir_all(&dir).unwrap();
            symlink(&victim, dir.join("dev-run.sh")).unwrap();

            let got = write_run_script(&share, overlay, "echo boot").unwrap();

            assert_eq!(got, dir);
            let script = std::fs::symlink_metadata(dir.join("dev-run.sh")).unwrap();
            assert!(script.is_file());
            assert_eq!(script.permissions().mode() & 0o777, 0o755);
            assert_eq!(
                std::fs::read_to_string(dir.join("dev-run.sh")).unwrap(),
                "echo boot"
            );
        }
        // Legacy: a symlinked `opt` is replaced, not followed.
        let share = t.path().join("share-opt");
        std::fs::create_dir_all(&share).unwrap();
        std::fs::create_dir_all(t.path().join("host/iii")).unwrap();
        symlink(t.path().join("host"), share.join("opt")).unwrap();
        write_run_script(&share, false, "echo boot").unwrap();
        assert!(!t.path().join("host/iii/dev-run.sh").exists());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "host");
        let victim_mode = std::fs::metadata(&victim).unwrap().permissions().mode();
        assert_eq!(victim_mode & 0o111, 0, "the host file was chmodded");
    }

    #[test]
    fn build_local_mounts_maps_project_to_workspace() {
        let mounts = build_local_mounts(Path::new("/abs/host/project"), "/workspace");
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].0, "/abs/host/project");
        assert_eq!(mounts[0].1, "/workspace");
    }

    #[test]
    fn build_local_mounts_overlay_uses_host_src_read_path() {
        // W1 copy-in mounts the host project at /mnt/host-src (read-only in
        // practice); the script copies it into a VM-local /workspace.
        let mounts = build_local_mounts(Path::new("/abs/host/project"), "/mnt/host-src");
        assert_eq!(mounts[0].0, "/abs/host/project");
        assert_eq!(mounts[0].1, "/mnt/host-src");
    }

    #[test]
    fn build_local_mounts_preserves_relative_path_string() {
        // Path stringification is lossy on non-UTF8, but for typical macOS/Linux
        // paths we round-trip exactly. Documents intended behavior.
        let mounts = build_local_mounts(Path::new("./relative/path"), "/workspace");
        assert_eq!(mounts[0].0, "./relative/path");
        assert_eq!(mounts[0].1, "/workspace");
    }

    #[test]
    fn build_libkrun_local_script_first_run() {
        let project = ProjectInfo {
            name: "test".to_string(),
            kind: Some("typescript".to_string()),
            setup_cmd: "apt-get install nodejs".to_string(),
            install_cmd: "npm install".to_string(),
            run_cmd: "node server.js".to_string(),
            env: HashMap::new(),
            base_image: None,
        };
        let script = build_libkrun_local_script(
            &project, false, /*is_bundle=*/ false, /*overlay=*/ false,
        );
        assert!(script.contains("apt-get install nodejs"));
        assert!(script.contains("npm install"));
        assert!(script.contains("node server.js"));
        assert!(script.contains(".iii-prepared"));
        assert!(script.contains("SRC=/mnt/host-src"));
        assert!(script.contains("touch /run/iii/source-ready"));
        assert!(!script.contains("mount --bind"));
        assert!(script.contains("node_modules"));
        // The guest receives real file operations over iii.source-sync.
        assert!(!script.contains("CHOKIDAR_USEPOLLING"));
        assert!(!script.contains("WATCHFILES_FORCE_POLLING"));
        assert!(!script.contains("TSC_WATCHFILE"));
    }

    #[test]
    fn build_libkrun_local_script_prepared() {
        let project = ProjectInfo {
            name: "test".to_string(),
            kind: Some("typescript".to_string()),
            setup_cmd: "apt-get install nodejs".to_string(),
            install_cmd: "npm install".to_string(),
            run_cmd: "node server.js".to_string(),
            env: HashMap::new(),
            base_image: None,
        };
        let script = build_libkrun_local_script(
            &project, true, /*is_bundle=*/ false, /*overlay=*/ false,
        );
        assert!(!script.contains("apt-get install nodejs"));
        assert!(!script.contains("npm install"));
        assert!(script.contains("node server.js"));
        assert!(script.contains("SRC=/mnt/host-src"));
        assert!(script.contains("touch /run/iii/source-ready"));
    }

    #[test]
    fn current_prepared_marker_matches_copy_in_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join(".iii-prepared");
        std::fs::write(&marker, PREPARED_MARKER_VERSION).unwrap();

        assert!(prepared_marker_matches_current_workspace(&marker));
    }

    #[test]
    fn build_libkrun_local_script_overlay_guards_install_in_guest() {
        let project = ProjectInfo {
            name: "test".to_string(),
            kind: Some("typescript".to_string()),
            setup_cmd: "apt-get install nodejs".to_string(),
            install_cmd: "npm install".to_string(),
            run_cmd: "node server.js".to_string(),
            env: HashMap::new(),
            base_image: None,
        };
        // Under overlay the host `prepared` flag is ignored: the block is
        // always emitted but guarded in-guest on the persistent marker, so the
        // install runs once and is skipped on later boots from the same upper.
        let script = build_libkrun_local_script(
            &project, /*prepared=*/ true, /*is_bundle=*/ false, true,
        );
        assert!(script.contains("cat /var/.iii-prepared"));
        assert!(script.contains(PREPARED_MARKER_VERSION));
        assert!(script.contains("rm -f /var/.iii-prepared"));
        assert!(script.contains("apt-get install nodejs"));
        assert!(script.contains("npm install"));
        // Durability: the marker write is followed by `sync` so deps + marker
        // flush to the ext4 upper before a fast worker exit tears the VM down.
        assert!(script.contains("printf '%s\\n'"));
        assert!(script.contains("sync\nfi"));
        assert!(script.contains("node server.js"));
        // Local workers copy host source from /mnt/host-src into /workspace
        // and do not bind-mount dependency directories.
        assert!(script.contains("SRC=/mnt/host-src"));
        assert!(script.contains("tar -cf -"));
        // Re-sync deletions: clear /workspace (minus dep dirs) before extract so
        // host-deleted files don't keep running on the persistent upper.
        assert!(script.contains("find /workspace -mindepth 1 -maxdepth 1"));
        assert!(script.contains("! -name node_modules"));
        // Host-untouched: the unmount is FATAL, not best-effort.
        assert!(script.contains("refusing to run with host source writable"));
        assert!(
            !script.contains("umount \"$SRC\" 2>/dev/null || true"),
            "overlay host-src unmount must be fatal, not swallowed"
        );
        assert!(
            !script.contains("mount --bind"),
            "overlay must not use the dep-dir bind loop (W1 copy-in replaces it)"
        );
        assert!(
            !script.contains("/var/iii/deps"),
            "overlay must not create host-visible dep bind targets"
        );
    }

    #[test]
    fn build_libkrun_local_script_legacy_uses_copy_in_for_native_events() {
        let project = ProjectInfo {
            name: "test".to_string(),
            kind: Some("typescript".to_string()),
            setup_cmd: String::new(),
            install_cmd: "npm install".to_string(),
            run_cmd: "node server.js".to_string(),
            env: HashMap::new(),
            base_image: None,
        };
        let script = build_libkrun_local_script(
            &project, /*prepared=*/ false, /*is_bundle=*/ false, false,
        );
        assert!(!script.contains("mount --bind"));
        assert!(!script.contains("/var/iii/deps"));
        assert!(script.contains("SRC=/mnt/host-src"));
        assert!(script.contains("touch /run/iii/source-ready"));
    }

    #[test]
    fn build_local_env_sets_engine_urls_and_worker_name() {
        let env = build_local_env("ws://localhost:49134", "my-worker", &HashMap::new());
        assert_eq!(env.get("III_ENGINE_URL").unwrap(), "ws://localhost:49134");
        assert_eq!(env.get("III_URL").unwrap(), "ws://localhost:49134");
        assert_eq!(env.get("III_WORKER_NAME").unwrap(), "my-worker");
    }

    #[test]
    fn build_local_env_preserves_custom_env() {
        let mut project_env = HashMap::new();
        project_env.insert("CUSTOM".to_string(), "value".to_string());
        let env = build_local_env("ws://localhost:49134", "my-worker", &project_env);
        assert_eq!(env.get("CUSTOM").unwrap(), "value");
        assert_eq!(env.get("III_ENGINE_URL").unwrap(), "ws://localhost:49134");
        assert_eq!(env.get("III_URL").unwrap(), "ws://localhost:49134");
    }

    #[test]
    fn build_local_env_worker_name_not_overridable_by_project_env() {
        let mut project_env = HashMap::new();
        project_env.insert("III_WORKER_NAME".to_string(), "impostor".to_string());
        let env = build_local_env("ws://localhost:49134", "my-worker", &project_env);
        assert_eq!(env.get("III_WORKER_NAME").unwrap(), "my-worker");
    }

    #[test]
    fn build_env_exports_excludes_engine_urls_and_worker_name() {
        let mut env = HashMap::new();
        env.insert(
            "III_ENGINE_URL".to_string(),
            "ws://localhost:49134".to_string(),
        );
        env.insert("III_URL".to_string(), "ws://localhost:49134".to_string());
        env.insert("III_WORKER_NAME".to_string(), "my-worker".to_string());
        env.insert("CUSTOM_VAR".to_string(), "custom-val".to_string());

        let exports = build_env_exports(&env);
        assert!(!exports.contains("III_ENGINE_URL"));
        assert!(!exports.contains("III_URL"));
        assert!(!exports.contains("III_WORKER_NAME"));
        assert!(exports.contains("CUSTOM_VAR='custom-val'"));
    }

    #[test]
    fn shell_escape_single_quote() {
        let result = shell_escape("it's");
        assert_eq!(result, "it'\\''s");
    }

    #[test]
    fn parse_manifest_resources_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let nonexistent = dir.path().join("nonexistent.yaml");
        let (cpus, memory) = parse_manifest_resources(&nonexistent);
        assert_eq!(cpus, 2);
        assert_eq!(memory, 2048);
    }

    #[test]
    fn parse_manifest_resources_custom() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("iii.worker.yaml");
        let yaml = r#"
name: resource-test
resources:
  cpus: 4
  memory: 4096
"#;
        std::fs::write(&manifest_path, yaml).unwrap();
        let (cpus, memory) = parse_manifest_resources(&manifest_path);
        assert_eq!(cpus, 4);
        assert_eq!(memory, 4096);
    }

    #[test]
    fn parse_manifest_resources_defaults_when_shape_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join(WORKER_MANIFEST);
        // Valid YAML, but `resources` is a scalar where the typed manifest
        // requires a mapping — WorkerManifest::from_value fails and the
        // lenient sizing probe must fall back to defaults.
        std::fs::write(&manifest_path, "name: w\nresources: lots\n").unwrap();

        let (cpus, memory) = parse_manifest_resources(&manifest_path);

        assert_eq!(cpus, 2);
        assert_eq!(memory, 2048);
    }

    fn test_override(state: &Path) -> VmOverride<'static> {
        VmOverride {
            state_dir: state.to_path_buf(),
            engine_url: "ws://localhost:49134",
            extra_env: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn start_rejects_manifest_with_unknown_keys_before_boot() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "name: w\nscripts:\n  start: \"node server.js\"\ntypo_key: 1\n";
        std::fs::write(dir.path().join(WORKER_MANIFEST), yaml).unwrap();

        let state = tempfile::tempdir().unwrap();
        let err = start_worker_impl(
            "iii-cov-test-start-unknown-key",
            dir.path().to_str().unwrap(),
            /*package=*/ None,
            test_override(state.path()),
            None,
        )
        .await
        .err()
        .expect("must fail before boot");

        assert!(err.contains("manifest validation failed at start"), "{err}");
    }

    #[tokio::test]
    async fn start_rejects_oversized_manifest_before_boot() {
        let dir = tempfile::tempdir().unwrap();
        let mut yaml = String::from("name: w\nscripts:\n  start: \"node server.js\"\n");
        let cap = super::super::project::MAX_LOCAL_MANIFEST_BYTES as usize;
        yaml.push_str(&"# pad\n".repeat(cap / 6 + 16));
        assert!(yaml.len() as u64 > super::super::project::MAX_LOCAL_MANIFEST_BYTES);
        std::fs::write(dir.path().join(WORKER_MANIFEST), yaml).unwrap();

        let state = tempfile::tempdir().unwrap();
        let err = start_worker_impl(
            "iii-cov-test-start-oversize",
            dir.path().to_str().unwrap(),
            /*package=*/ None,
            test_override(state.path()),
            None,
        )
        .await
        .err()
        .expect("must fail before boot");

        assert!(err.contains("is capped at"), "{err}");
    }

    #[tokio::test]
    async fn start_rejects_a_missing_manifest_before_boot() {
        let dir = tempfile::tempdir().unwrap();

        let state = tempfile::tempdir().unwrap();
        let err = start_worker_impl(
            "iii-cov-test-start-no-manifest",
            dir.path().to_str().unwrap(),
            /*package=*/ None,
            test_override(state.path()),
            None,
        )
        .await
        .err()
        .expect("must fail before boot");

        assert!(err.contains("iii.worker.yaml does not exist"), "{err}");
    }

    #[tokio::test]
    async fn start_passes_key_validation_then_rejects_unrecognized_kind() {
        // Keys are all known (runtime.kind is deprecated, not unknown), so the
        // strict start-time validator falls through; the run then fails at
        // ProjectInfo::validate on the unsupported kind — still pre-boot.
        let dir = tempfile::tempdir().unwrap();
        let yaml = "name: w\nruntime:\n  kind: cobol\n\
                    scripts:\n  install: \"true\"\n  start: \"node server.js\"\n";
        std::fs::write(dir.path().join(WORKER_MANIFEST), yaml).unwrap();

        let state = tempfile::tempdir().unwrap();
        let err = start_worker_impl(
            "iii-cov-test-start-bad-kind",
            dir.path().to_str().unwrap(),
            /*package=*/ None,
            test_override(state.path()),
            None,
        )
        .await
        .err()
        .expect("must fail before boot");

        assert!(err.contains("unrecognized runtime kind 'cobol'"), "{err}");
    }

    #[tokio::test]
    async fn bundle_start_skips_local_validation_then_rejects_unrecognized_kind() {
        // A package takes validate_bundle_manifest (which allows the
        // deprecated runtime.kind) and skips the local strict-validation
        // block; the run then fails pre-boot at ProjectInfo::validate. The
        // manifest name matches the package, not the container key.
        let dir = tempfile::tempdir().unwrap();
        let yaml = "name: iii-cov-test-bundle-kind\nruntime:\n  kind: cobol\n\
                    scripts:\n  start: \"node bundle.js\"\n";
        std::fs::write(dir.path().join(WORKER_MANIFEST), yaml).unwrap();

        let state = tempfile::tempdir().unwrap();
        let err = start_worker_impl(
            "compose-key",
            dir.path().to_str().unwrap(),
            Some("iii-cov-test-bundle-kind"),
            test_override(state.path()),
            None,
        )
        .await
        .err()
        .expect("must fail before boot");

        assert!(err.contains("unrecognized runtime kind 'cobol'"), "{err}");
    }

    /// Regression test for the "managed_dir exists but is missing /bin"
    /// failure mode. A half-populated managed_dir — created by a prior
    /// iii-init boot that mkdir'd /dev/proc/sys but died before the
    /// rootfs was cloned, or by an older codepath that placed the
    /// rootfs at `managed_dir/rootfs/` rather than at `managed_dir/` —
    /// must be detected as "needs re-clone" by `needs_rootfs_clone`.
    ///
    /// The pre-fix guard (`!managed_dir.exists()`) skipped the clone
    /// and let the VM boot against an FHS-less rootfs, surfacing as a
    /// confusing `supervisor spawn_initial failed: No such file or
    /// directory (os error 2)` from iii-init because `/bin/sh` didn't
    /// exist. This test calls the extracted helper so a refactor that
    /// weakens the predicate back to `!managed_dir.exists()` — or any
    /// other too-permissive check — fails here.
    #[test]
    fn needs_rootfs_clone_true_for_half_populated_managed_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let managed = tmp.path().join("todo-worker-python");

        // Simulate iii-init-style runtime dirs (created on prior boot)
        // + a stale /rootfs subdir from an older codepath — the exact
        // layout the user hit in the wild.
        for d in [
            "dev",
            "proc",
            "sys",
            "etc",
            "tmp",
            "run",
            "opt",
            "workspace",
            "rootfs/bin",
            "rootfs/usr",
        ] {
            std::fs::create_dir_all(managed.join(d)).unwrap();
        }

        // Dir exists — the old too-weak guard would skip the clone.
        assert!(managed.exists());
        // But the helper must flag this as "rootfs never got populated
        // here, must re-clone" — otherwise the VM boots against an
        // FHS-less tree and iii-init dies trying to exec /bin/sh.
        assert!(
            needs_rootfs_clone(&managed),
            "half-populated managed_dir must register as `needs clone`"
        );
    }

    /// Complement: a fully-cloned managed_dir (with `/bin` present)
    /// must be recognized as ready so we don't nuke a healthy sandbox
    /// on every start and pay the re-clone cost. Exercises the helper
    /// directly for the same reason as the sibling test above.
    #[test]
    fn needs_rootfs_clone_false_for_fully_cloned_managed_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let managed = tmp.path().join("todo-worker-python");
        std::fs::create_dir_all(managed.join("bin")).unwrap();
        std::fs::create_dir_all(managed.join("usr")).unwrap();
        assert!(
            !needs_rootfs_clone(&managed),
            "fully-cloned managed_dir must NOT trigger a re-clone"
        );
    }

    /// Absence case: `needs_rootfs_clone(nonexistent_path)` must be true.
    /// Covers the first-start path where the worker's state dir has never
    /// been created.
    #[test]
    fn needs_rootfs_clone_true_when_managed_dir_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let managed = tmp.path().join("never-started");
        assert!(!managed.exists());
        assert!(
            needs_rootfs_clone(&managed),
            "absent managed_dir must register as `needs clone`"
        );
    }
}
