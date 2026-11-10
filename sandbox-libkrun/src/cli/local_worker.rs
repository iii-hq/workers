// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Local-path and bundle worker VMs: `local_vm_command` and
//! `bundle_vm_command` build the `__vm-boot` command for a worker directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use super::project::{ProjectInfo, WORKER_MANIFEST, project_from_doc};
use super::rootfs::clone_rootfs;
use super::source_watcher::{IGNORED_DIR_NAMES, ROOT_IGNORED_DIR_NAMES};

const PREPARED_MARKER_VERSION: &str = "workspace-copy-in-v1";

/// The worker command. A valid base image may lack bash (`node:20-alpine`);
/// its `sh` then runs the script, whose only bash-ism, `set -o pipefail`,
/// busybox ash supports. Images with bash keep running the script under it,
/// as their setup/install commands may rely on bash syntax.
const RUN_SCRIPT_CMD: &str =
    "command -v bash >/dev/null && exec bash /opt/iii/dev-run.sh; exec sh /opt/iii/dev-run.sh";

// What stays out of a `path://` worker's `/workspace` copy is one list: the
// names the source watcher ignores, plus `.iii`. `IGNORED_DIR_NAMES`
// (dependency, cache, build-output, VCS and editor dirs, Compose's `.build` and
// `.workers`) match at any depth; `ROOT_IGNORED_DIR_NAMES` (build output that
// projects also use for nested source) match only as a directory at the
// worker's root. The run copies and syncs by it; the build copies by it too,
// but for `BUILD_COPIES`.

/// What a build copies although a run leaves it out: Go vendors source in
/// `vendor/`, and a PHP `composer install` in the build rewrites it anyway.
const BUILD_COPIES: &[&str] = &["vendor"];

/// Where a guest keeps the manifests of its `/workspace` copies: on the data
/// disk, outside `/workspace`. A build's copy writes [`BUILD_MANIFEST`], which
/// ships in its image; a run's copy writes [`RUN_MANIFEST`].
const WORKSPACE_STATE: &str = "/var/lib/iii";
const BUILD_MANIFEST: &str = "build.manifest";
const RUN_MANIFEST: &str = "workspace.manifest";

/// The names a copy leaves out at any depth: every any-depth name of the list
/// but `keep`, and `.iii`, Compose's state, which holds the project's token.
/// The root-only names are left to [`copy_in_script`], which knows whether
/// they are dirs.
fn left_out<'a>(keep: &'a [&str]) -> impl Iterator<Item = &'static str> + 'a {
    IGNORED_DIR_NAMES
        .iter()
        .copied()
        .filter(|name| !keep.contains(name))
        .chain([".iii"])
}

/// `--exclude` flags for [`left_out`].
fn source_excludes(keep: &[&str]) -> String {
    left_out(keep)
        .map(|name| format!("--exclude={name}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The `tar --exclude` flags of a run's copy.
static SOURCE_EXCLUDES: LazyLock<String> = LazyLock::new(|| source_excludes(&[]));
/// The `tar --exclude` flags of a build's copy.
static BUILD_SOURCE_EXCLUDES: LazyLock<String> = LazyLock::new(|| source_excludes(BUILD_COPIES));

/// The copy of a `path://` worker's code from `$SRC` into `$WS`, with
/// `excludes`. A root-only name stays out only as a directory (`./build/`), so a
/// root `build` script still comes along: `--exclude=./<name>` matches the
/// root's entry only. What tar extracted, one path per line as `tar -v` prints
/// it (a directory with a trailing `/`), replaces `$IIISTATE/<manifest>` once
/// the copy succeeded: the next boot's [`workspace_cleanup`] reads it.
fn copy_in_script(excludes: &str, manifest: &str) -> String {
    let root_only = ROOT_IGNORED_DIR_NAMES.join(" ");
    format!(
        r#"ROOT_EXCLUDES=
for name in {root_only}; do
  if [ -d "$SRC/$name" ] && [ ! -L "$SRC/$name" ]; then ROOT_EXCLUDES="$ROOT_EXCLUDES --exclude=./$name"; fi
done
mkdir -p "$IIISTATE"
( set -o pipefail; ( cd "$SRC" && tar -cf - {excludes} $ROOT_EXCLUDES . ) | ( cd "$WS" && tar -xpvf - ) > "$IIISTATE/{manifest}.tmp" ) || {{ echo "iii: ERROR workspace copy-in failed" >&2; exit 1; }}
mv -f "$IIISTATE/{manifest}.tmp" "$IIISTATE/{manifest}""#
    )
}

/// A run's copy, after a build installed its dependencies.
static RUN_COPY: LazyLock<String> =
    LazyLock::new(|| copy_in_script(&SOURCE_EXCLUDES, RUN_MANIFEST));
/// The copy of a run that installs in the guest (`worker::start` without
/// `build`): a build's list, as it installs after it.
static INSTALL_COPY: LazyLock<String> =
    LazyLock::new(|| copy_in_script(&BUILD_SOURCE_EXCLUDES, RUN_MANIFEST));
/// A build's copy: its manifest ships in the image.
static BUILD_COPY: LazyLock<String> =
    LazyLock::new(|| copy_in_script(&BUILD_SOURCE_EXCLUDES, BUILD_MANIFEST));

/// Re-sync deletions: tar -x only adds and overwrites, and `/workspace` keeps
/// what earlier copies left (on the data disk, or in the build's image). Before
/// the copy, remove the entries of the last copies' manifests (the build's and
/// the last run's) that the host no longer has, or has as another kind (a file
/// where a directory was), deepest first. Nothing else goes: what the build
/// installed and what the worker wrote are in no manifest, whatever their name
/// or depth.
///
/// - An entry with a component this boot's copy leaves out (all but `keep` of
///   [`left_out`]) is never touched: a run neither copies, syncs nor cleans
///   those names, so the `vendor/` a build copied stays whole until the next
///   build.
/// - Nothing is removed, emptied or moved through a symlink: `$WS` and the
///   entry's parent must be chains of real directories, the parent inside
///   `$WS`. A link the worker, the install or the sync put there is skipped.
/// - A directory goes only once empty; one that the host turned into a file
///   but that still holds such things moves aside, to the first free
///   `<name>.iii-kept.<n>`, to make way for it.
/// - Only a line `tar -v` printed as one plain name counts: it starts with
///   `./`, is not continued on the next line (busybox prints a newline raw)
///   and has no backslash (GNU tar escapes newlines and backslashes). A name
///   with a newline or a backslash is never cleaned.
///
/// Without a manifest (a disk an older adapter wrote) there is nothing to go
/// by: what the host deleted before this boot stays until the worker is
/// destroyed. A `.iii` an older adapter copied in goes in any case: it holds
/// the project's token.
fn workspace_cleanup(keep: &[&str]) -> String {
    let skip = left_out(keep)
        .map(|name| format!("*/\"{name}\"/*"))
        .collect::<Vec<_>>()
        .join("|");
    WORKSPACE_CLEANUP_SCRIPT.replace("@SKIP@", &skip)
}

const WORKSPACE_CLEANUP_SCRIPT: &str = r#"find "$WS" -mindepth 1 -name .iii -prune -exec rm -rf {} + 2>/dev/null || true
if [ -f "$IIISTATE/workspace.manifest" ] || [ -f "$IIISTATE/build.manifest" ]; then
  if [ "$(cd -P "$WS" 2>/dev/null && pwd -P)" != "$WS" ]; then
    echo "iii: $WS is not a real directory; skipping the workspace cleanup" >&2
  else
    safe_parent=
    for manifest in "$IIISTATE/workspace.manifest" "$IIISTATE/build.manifest"; do
      if [ -f "$manifest" ]; then
        awk '/^\.\// { if (one) print name; name = $0; one = 1; next } { one = 0 } END { if (one) print name }' "$manifest"
      fi
    done | LC_ALL=C sort -ru |
    while IFS= read -r entry; do
      case "$entry" in ./*) ;; *) continue ;; esac
      case "$entry" in *\\*) continue ;; esac
      rel=${entry#./}
      rel=${rel%/}
      case "$rel" in ''|.|..|../*|*/..|*/../*) continue ;; esac
      case "/$rel/" in @SKIP@) continue ;; esac
      ws="$WS/$rel"
      host="$SRC/$rel"
      if [ "$entry" != "${entry%/}" ]; then
        if [ -d "$host" ] && [ ! -L "$host" ]; then continue; fi
        if [ ! -d "$ws" ] || [ -L "$ws" ]; then continue; fi
      else
        if [ -e "$host" ] || [ -L "$host" ]; then
          if [ ! -d "$host" ] || [ -L "$host" ]; then continue; fi
        fi
        if [ ! -L "$ws" ] && { [ ! -e "$ws" ] || [ -d "$ws" ]; }; then continue; fi
      fi
      parent=${ws%/*}
      if [ "$parent" != "$safe_parent" ]; then
        [ "$(cd -P "$parent" 2>/dev/null && pwd -P)" = "$parent" ] || continue
        safe_parent=$parent
      fi
      if [ "$entry" = "${entry%/}" ]; then
        rm -f "$ws" || true
      elif rmdir "$ws" 2>/dev/null; then
        safe_parent=
      elif [ -e "$host" ] || [ -L "$host" ]; then
        n=1
        while [ -e "$ws.iii-kept.$n" ] || [ -L "$ws.iii-kept.$n" ]; do n=$((n + 1)); done
        mv "$ws" "$ws.iii-kept.$n" && echo "iii: $rel is no longer a directory on the host; what the build or the worker left in it is at $rel.iii-kept.$n" >&2
        safe_parent=
      fi
    done || true
  fi
elif [ -n "$(ls -A "$WS" 2>/dev/null)" ]; then
  echo "iii: no workspace manifest yet (a disk an older adapter wrote): what the host deleted before this boot stays in $WS until the worker is destroyed" >&2
fi"#;

/// The cleanup before a run's copy.
static RUN_CLEANUP: LazyLock<String> = LazyLock::new(|| workspace_cleanup(&[]));
/// The cleanup before the copy of a run that installs in the guest: it copies
/// as a build does, so it cleans as one would.
static INSTALL_CLEANUP: LazyLock<String> = LazyLock::new(|| workspace_cleanup(BUILD_COPIES));

// ──────────────────────────────────────────────────────────────────────────────
// Shared helpers
// ──────────────────────────────────────────────────────────────────────────────

/// In legacy mode the marker is on the guest root, so the guest may have
/// planted a symlink (to `/dev/zero`, say), a FIFO or a huge file there:
/// never follow it, never block on it, and read only a marker's worth.
fn prepared_marker_matches_current_workspace(marker: &Path) -> bool {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;

    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(marker)
    else {
        return false;
    };
    let mut contents = String::new();
    file.metadata().is_ok_and(|meta| meta.is_file())
        && file
            .take(PREPARED_MARKER_VERSION.len() as u64 + 1)
            .read_to_string(&mut contents)
            .is_ok()
        && contents.trim() == PREPARED_MARKER_VERSION
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
/// (2 vCPUs, 2048 MiB) — strictness lives in the start validation, not in
/// this sizing probe. Backed by the typed
/// [`super::worker_manifest::WorkerManifest`] so validation and sizing read
/// resources through one schema.
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

/// How a boot script provisions (`scripts.setup` + `scripts.install`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Provision {
    /// Legacy clone: the host saw the marker, `prepared` skips the block.
    Host { prepared: bool },
    /// Overlay: the marker is on the ext4 upper, so the guest checks it.
    InGuest,
    /// The root is a build's image: nothing to install.
    Image,
}

/// The shell header every boot script starts with. The script never re-exports
/// the manifest env: the VM env already holds it merged with the caller's
/// overrides and the gateway rewrite. The system dirs go after PATH so a PATH
/// from that env keeps precedence.
fn script_header() -> Vec<String> {
    vec![
        "set -e".to_string(),
        "export HOME=${HOME:-/root}".to_string(),
        "export PATH=${PATH:+$PATH:}/usr/local/bin:/usr/bin:/bin".to_string(),
        "export LANG=${LANG:-C.UTF-8}".to_string(),
    ]
}

/// The host source share at `$SRC`, checked before anything is copied from it.
/// The mountpoint check guards against iii-init's mount_virtiofs_shares()
/// swallowing a mount failure as a warning (leaving the path
/// existing-but-unmounted), which would otherwise silently copy nothing.
const SOURCE_SHARE_CHECK: &str = r#"SRC=/mnt/host-src
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
fi"#;

/// Build the boot script that `iii-init` will exec as the worker command.
///
/// No supervisor wrapping: the in-VM `iii-init` binary (PID 1) starts
/// and reaps the worker itself (see `iii_init::supervisor`).
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
    let provision = if overlay {
        Provision::InGuest
    } else {
        Provision::Host { prepared }
    };
    boot_script(project, is_bundle, provision)
}

/// The boot script of a VM whose root is a build's image (`worker::start`
/// with `build`): no `scripts.setup` or `scripts.install`, they ran in the
/// build. A bundle's code is in the image; a local worker's still syncs in.
pub fn build_image_run_script(project: &ProjectInfo, is_bundle: bool) -> String {
    boot_script(project, is_bundle, Provision::Image)
}

fn boot_script(project: &ProjectInfo, is_bundle: bool, provision: Provision) -> String {
    let mut parts = script_header();
    // A guest that installs gets what a build's copy gets (`vendor/` too); a
    // run of a build's image the run's list.
    let (cleanup, copy) = match provision {
        Provision::Image => (RUN_CLEANUP.as_str(), RUN_COPY.as_str()),
        Provision::Host { .. } | Provision::InGuest => {
            (INSTALL_CLEANUP.as_str(), INSTALL_COPY.as_str())
        }
    };

    // Workspace strategy:
    //
    //  - BUNDLE: the immutable install dir is mounted live at /workspace via
    //    virtiofs; vendored deps ship inside it, so no dep handling is needed.
    //    A build's image holds the bundle's code at /workspace instead.
    //
    //  - LOCAL: the host project is mounted temporarily at /mnt/host-src and
    //    copied into /workspace. The share is detached before user code runs.
    //    Later changes arrive over `iii.source-sync` and are written by
    //    iii-init, which makes the guest kernel emit native inotify events.
    //    Dependency and build directories remain VM-local and are excluded
    //    from both the initial copy and later synchronization.
    if is_bundle && provision == Provision::Image {
        parts.push(
            r#"cd /workspace
echo "iii: workspace ready (bundle worker; code from the build image)" >&2"#
                .to_string(),
        );
    } else if is_bundle {
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
        parts.push(SOURCE_SHARE_CHECK.to_string());
        parts.push(format!("WS=/workspace\nIIISTATE={WORKSPACE_STATE}"));
        parts.push(cleanup.to_string());
        // Copy host source into the VM-local /workspace, without the dirs of
        // the list, so what the build or the guest installed persists across
        // a re-copy and host clutter isn't pulled in. Only /mnt/host-src is
        // read; the host project is never written. .iii is Compose's state,
        // which holds the project's token: the guest must never get a copy.
        parts.push(copy.to_string());
        parts.push(
            r#"# Enforce "host project untouched": detach the initial source share before
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
    //   - image: the build ran them; a run never installs.
    match provision {
        Provision::InGuest => {
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
        }
        Provision::Host { prepared: false } => {
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
        Provision::Host { prepared: true } | Provision::Image => {}
    }

    // Exec the user command directly; iii-init supervises the one
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
    parts.push(format!("exec /bin/sh -c {quoted_run_cmd}"));
    parts.join("\n")
}

/// The script of a build VM (`worker::build`): copy the worker's code into
/// `/workspace`, run `scripts.setup` and `scripts.install` there, and exit.
/// When it exits 0, iii-init exports the merged root for the image. A local
/// worker's copy leaves out what its run's copy leaves out, but `vendor/`
/// ([`BUILD_COPIES`]); a bundle's is the whole package but Compose's state.
pub fn build_image_script(project: &ProjectInfo, is_bundle: bool) -> String {
    let mut parts = script_header();
    parts.push(SOURCE_SHARE_CHECK.to_string());
    parts.push("echo \"iii: build: copying the worker's code into /workspace\" >&2".to_string());
    if is_bundle {
        // The whole package is the bundle's code; a run never copies it again.
        parts.push(
            r#"( set -o pipefail; ( cd "$SRC" && tar -cf - --exclude=.iii . ) | ( cd /workspace && tar -xpf - ) ) || { echo "iii: ERROR workspace copy-in failed" >&2; exit 1; }"#
                .to_string(),
        );
    } else {
        // Its manifest ships in the image, for the first run's cleanup.
        parts.push(format!("WS=/workspace\nIIISTATE={WORKSPACE_STATE}"));
        parts.push(BUILD_COPY.to_string());
    }
    parts.push(
        r#"cd /
umount "$SRC" || { echo "iii: ERROR could not detach $SRC" >&2; exit 1; }
cd /workspace
echo $$ > /sys/fs/cgroup/worker/cgroup.procs 2>/dev/null || true"#
            .to_string(),
    );
    if !project.setup_cmd.is_empty() {
        parts.push("echo \"iii: build: running scripts.setup\" >&2".to_string());
        parts.push(project.setup_cmd.clone());
    }
    if !project.install_cmd.is_empty() {
        parts.push("echo \"iii: build: running scripts.install\" >&2".to_string());
        parts.push(project.install_cmd.clone());
    }
    parts.push("echo \"iii: build: scripts done\" >&2".to_string());
    parts.join("\n")
}

pub fn build_local_env(
    engine_url: &str,
    worker_name: &str,
    project_env: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("III_ENGINE_URL".to_string(), engine_url.to_string());
    env.insert("III_URL".to_string(), engine_url.to_string());
    // The caller's worker name, so the SDK registers under it instead of
    // its `hostname:pid` fallback.
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
fn validate_compose_base_image(
    doc: &serde_yaml::Value,
    manifest_path: &Path,
) -> Result<(), String> {
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
/// them exit, and captures their output — and therefore needs the handle.
/// Every bundle gate applies: the
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
    /// Per-caller VM state: rootfs (or overlay trampoline) and boot script.
    pub state_dir: PathBuf,
    /// Engine URL the guest connects to.
    pub engine_url: &'a str,
    /// Merged over the manifest's own env, and under the reserved keys
    /// [`build_local_env`] owns.
    pub extra_env: HashMap<String, String>,
    /// Boot this build instead of the base image, with no setup or install.
    pub image: Option<BootImage>,
}

/// A build's image a VM boots (`worker::start` with `build`).
pub struct BootImage {
    /// `<build_dir>/image.erofs`, the overlay's read-only lower.
    pub image: PathBuf,
    /// The VM's persistent data disk, the overlay's upper.
    pub upper: PathBuf,
}

/// iii-init's build-mode switch (`iii_init::export`). Only the host sets it, on
/// a build VM: a caller's env never carries it into a VM.
const BUILD_EXPORT_ENV: &str = "III_BUILD_EXPORT";
/// Where a build VM's iii-init writes the root: the `/opt/iii` share, the
/// trampoline's `runtime/` on the host.
const BUILD_EXPORT_GUEST: &str = "/opt/iii/root.tar";
const BUILD_EXPORT_HOST: &str = "runtime/root.tar";

/// A build VM (`worker::build`), built but not spawned.
pub struct BuildVm {
    pub command: super::worker_manager::libkrun::VmCommand,
    /// The build VM's trampoline root, where iii-init's tar lands once the
    /// build script exits 0; see [`open_build_export`].
    pub root: PathBuf,
    /// The base image the build starts from, as the rootfs cache keys it.
    pub reference: String,
    /// Its cached rootfs; the image takes its `.oci-config.json` from there.
    pub base_rootfs: PathBuf,
}

/// The tar a build VM exported under its trampoline `root` ([`BuildVm::root`]).
/// That share was the guest's, so no component of the path is followed through
/// a symlink, and it must be a regular file.
pub fn open_build_export(root: &Path) -> std::io::Result<std::fs::File> {
    super::worker_manager::oci::open_rootfs_file_no_follow(root, Path::new(BUILD_EXPORT_HOST))
}

/// Builds the build VM of a worker: the base image (pulled if the cache lacks
/// it) as the lower, a fresh upper in `scratch`, the worker's code shared
/// read-only at `/mnt/host-src`, and [`build_image_script`] to run. Nothing is
/// spawned. `env` is the caller's `guest_env`, over the manifest's own env.
pub(crate) async fn image_build_command(
    worker_dir: &Path,
    package: Option<&str>,
    run_override: Option<&str>,
    scratch: &Path,
    env: HashMap<String, String>,
) -> Result<BuildVm, String> {
    use super::worker_manager::oci;

    let (project, kind) = validate_source(worker_dir, package, run_override)?;
    let is_bundle = package.is_some();
    if let Err(e) = super::firmware::provision::ensure_libkrunfw().await {
        tracing::warn!(error = %e, "failed to ensure libkrunfw");
    }
    if !super::worker_manager::libkrun::libkrun_available() {
        return Err("No runtime available.\n  \
             Rebuild with --features embed-libkrunfw or place libkrunfw in ~/.iii/lib/"
            .to_string());
    }
    let reference = oci::base_image_ref(&kind, project.base_image.as_deref());
    let base_rootfs = oci::prepare_rootfs(&kind, project.base_image.as_deref())
        .await
        .map_err(|e| format!("cannot pull the base image {reference}: {e:#}"))?;
    let lower = {
        let base = base_rootfs.clone();
        tokio::task::spawn_blocking(move || crate::cli::erofs::ensure_base_erofs(&base))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("failed to build base erofs: {e}"))?
    };

    let root = scratch.join("root");
    std::fs::create_dir_all(&root)
        .and_then(|()| oci::ensure_rootfs_dir_no_follow(&root, "workspace").map(drop))
        .map_err(|e| format!("cannot create the build VM's dir {}: {e}", root.display()))?;
    let upper = crate::cli::upper::ensure_upper_ext4(scratch)
        .map_err(|e| format!("failed to materialize the build's upper: {e}"))?;
    let script_dir = write_run_script(&root, true, &build_image_script(&project, is_bundle))
        .map_err(|e| format!("Failed to write the build script: {e}"))?;

    let mut env_all = project.env.clone();
    env_all.extend(env);
    for (key, value) in oci::read_oci_env(&base_rootfs) {
        env_all.entry(key).or_insert(value);
    }
    env_all.insert(BUILD_EXPORT_ENV.to_string(), BUILD_EXPORT_GUEST.to_string());

    let (vcpus, ram) = vm_resources(worker_dir, is_bundle, &project.name);
    let mounts = [
        (
            worker_dir.to_string_lossy().into_owned(),
            "/mnt/host-src".to_string(),
        ),
        (
            script_dir.to_string_lossy().into_owned(),
            "/opt/iii".to_string(),
        ),
    ];
    let command = super::worker_manager::libkrun::build_vm_command(
        "/bin/sh",
        &["-c".to_string(), RUN_SCRIPT_CMD.to_string()],
        env_all,
        vcpus,
        ram,
        root.clone(),
        Some(lower),
        "overlay".to_string(),
        Some(upper),
        &mounts,
        None,
        Some(worker_dir),
    )?;
    debug_assert_eq!(script_dir.join("root.tar"), root.join(BUILD_EXPORT_HOST));
    Ok(BuildVm {
        command,
        root,
        reference,
        base_rootfs,
    })
}

/// The env the base image's config sets, from the `.oci-config.json` a build
/// put into its image; empty when it has none.
fn image_oci_env(image: &Path) -> Vec<(String, String)> {
    match crate::cli::erofs::reader::read_file_from_erofs(image, "/.oci-config.json") {
        Ok(bytes) => {
            super::worker_manager::oci::oci_env_from_config(&String::from_utf8_lossy(&bytes))
        }
        Err(e) => {
            tracing::warn!(image = %image.display(), error = %e, "the build image has no OCI config");
            Vec::new()
        }
    }
}

/// The VM of a build (`worker::start` with `build`): the image as the lower,
/// the VM's data disk as the upper, and [`build_image_run_script`]. A local
/// worker's source still syncs in; a bundle's code is in the image.
fn image_run_command(
    worker_name: &str,
    project_path: &Path,
    project: &ProjectInfo,
    is_bundle: bool,
    over: VmOverride<'_>,
) -> Result<super::worker_manager::libkrun::VmCommand, String> {
    let BootImage { image, upper } = over.image.expect("a build to boot");
    if !crate::cli::overlay::overlay_active() {
        return Err(
            "booting a build needs the overlay rootfs mode; this adapter runs in \
             legacy mode (III_ROOTFS_MODE=legacy, or no embedded iii-init)"
                .to_string(),
        );
    }
    let managed_dir = over.state_dir;
    std::fs::create_dir_all(&managed_dir)
        .and_then(|()| {
            super::worker_manager::oci::ensure_rootfs_dir_no_follow(&managed_dir, "workspace")
                .map(drop)
        })
        .map_err(|e| format!("Failed to create worker dir {}: {e}", managed_dir.display()))?;

    let mut combined_project_env = project.env.clone();
    combined_project_env.extend(over.extra_env);
    let mut env = build_local_env(over.engine_url, worker_name, &combined_project_env);
    for (key, value) in image_oci_env(&image) {
        env.entry(key).or_insert(value);
    }
    env.remove(BUILD_EXPORT_ENV);

    let script_dir = write_run_script(
        &managed_dir,
        true,
        &build_image_run_script(project, is_bundle),
    )
    .map_err(|e| format!("Failed to write run script: {e}"))?;
    let (vcpus, ram) = vm_resources(project_path, is_bundle, worker_name);
    let mut mounts = if is_bundle {
        Vec::new()
    } else {
        build_local_mounts(project_path, "/mnt/host-src")
    };
    mounts.push((
        script_dir.to_string_lossy().into_owned(),
        "/opt/iii".to_string(),
    ));
    super::worker_manager::libkrun::build_vm_command(
        "/bin/sh",
        &["-c".to_string(), RUN_SCRIPT_CMD.to_string()],
        env,
        vcpus,
        ram,
        managed_dir,
        Some(image),
        "overlay".to_string(),
        Some(upper),
        &mounts,
        (!is_bundle).then_some(project_path),
        None,
    )
}

/// vCPUs and MiB of RAM: a bundle's clamped, as its manifest is
/// publisher-controlled, a local worker's as its manifest asks.
fn vm_resources(project_path: &Path, is_bundle: bool, worker_name: &str) -> (u32, u32) {
    if !is_bundle {
        return parse_manifest_resources(&project_path.join(WORKER_MANIFEST));
    }
    // Strict clamp at start-time so a publisher-controlled manifest
    // can't boot a libkrun guest with attacker-chosen resources:
    // the VM boot re-reads the manifest from the install dir, which
    // may have changed since install.
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
    // we touch the publisher-controlled YAML via project_from_doc.
    // Whatever checked the bundle at install time, the
    // install dir is on the host filesystem — any local-FS-write
    // attacker (or a future bug that lets a bundle write to its own
    // install dir) could modify the iii.worker.yaml in that install dir
    // (Compose's package cache) between install and start to introduce
    // scripts.setup, scripts.install, or runtime.base_image. The permissive
    // project_from_doc path below would honor those fields. The
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
    // the engine via the permissive load path below. The manifest is read
    // once, so the doc that passes these checks is the one loaded.
    let manifest_path = project_path.join(WORKER_MANIFEST);
    let doc = super::project::read_manifest_doc(&manifest_path)?
        .ok_or_else(|| format!("{} does not exist", manifest_path.display()))?;
    if package.is_none()
        && let Err(e) = super::project::validate_manifest_keys(&doc, &manifest_path)
    {
        return Err(format!("manifest validation failed at start: {}", e));
    }

    // 2. Load the manifest
    let mut project = project_from_doc(&doc, &manifest_path).ok_or_else(|| {
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
        validate_compose_base_image(&doc, &manifest_path)?;
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
    if over.image.is_some() {
        return image_run_command(worker_name, project_path, &project, is_bundle, over);
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
    // A build VM's switch: a caller's env must not turn a run into an export.
    env.remove(BUILD_EXPORT_ENV);

    // 7. Build boot script. No supervisor wrap is emitted — the in-VM
    //    `iii-init` binary absorbs that role (see iii_init::supervisor).
    let script = build_libkrun_local_script(&project, is_prepared, is_bundle, overlay);

    // Both modes run `/opt/iii/dev-run.sh`; only the SOURCE of that file
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
    // tiny (RUN_SCRIPT_CMD) keeps the env within that budget
    // for both modes.
    let script_dir = write_run_script(&managed_dir, overlay, &script)
        .map_err(|e| format!("Failed to write run script: {}", e))?;
    // The script sets up the workspace; no pre-cd needed (and /workspace is
    // empty until virtiofs mounts, so cd-before-exec would be racy).
    let exec_path = "/bin/sh";
    let args = vec!["-c".to_string(), RUN_SCRIPT_CMD.to_string()];

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
    let (vcpus, ram) = vm_resources(project_path, is_bundle, worker_name);

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
        None,
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

    /// The build VM's share was the guest's: neither a symlink planted at the
    /// tar nor one at `runtime` is read through.
    #[test]
    fn a_build_export_is_never_read_through_a_guest_symlink() {
        use std::os::unix::fs::symlink;
        let t = tempfile::tempdir().unwrap();
        let host = t.path().join("host");
        std::fs::create_dir_all(&host).unwrap();
        std::fs::write(host.join("root.tar"), "host file").unwrap();
        let root = t.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        symlink(&host, root.join("runtime")).unwrap();
        assert!(
            open_build_export(&root).is_err(),
            "followed runtime -> host"
        );
        std::fs::remove_file(root.join("runtime")).unwrap();
        std::fs::create_dir(root.join("runtime")).unwrap();
        symlink(host.join("root.tar"), root.join("runtime/root.tar")).unwrap();
        assert!(
            open_build_export(&root).is_err(),
            "followed root.tar -> host"
        );
        std::fs::remove_file(root.join("runtime/root.tar")).unwrap();
        std::fs::write(root.join("runtime/root.tar"), "tar").unwrap();
        assert!(open_build_export(&root).is_ok());
    }

    fn project_with(setup: &str, install: &str) -> ProjectInfo {
        ProjectInfo {
            name: "test".to_string(),
            kind: Some("typescript".to_string()),
            setup_cmd: setup.to_string(),
            install_cmd: install.to_string(),
            run_cmd: "node server.js".to_string(),
            env: HashMap::new(),
            base_image: None,
        }
    }

    /// A build VM copies the code in, installs, and exits for iii-init to
    /// export: no worker runs, no marker, no source sync.
    #[test]
    fn the_build_script_installs_and_exits() {
        let project = project_with("apt-get install -y git", "npm ci");
        let script = build_image_script(&project, /*is_bundle=*/ false);
        assert!(script.contains("apt-get install -y git\necho"));
        assert!(script.contains("npm ci"));
        assert!(script.contains(BUILD_SOURCE_EXCLUDES.as_str()));
        assert!(script.contains("umount \"$SRC\""));
        assert!(!script.contains("exec /bin/sh -c"), "{script}");
        assert!(!script.contains(".iii-prepared"));
        assert!(!script.contains("source-ready"));
        assert!(script.find("apt-get") < script.find("npm ci"));
        // A bundle's whole package is its code, vendored deps included.
        let bundle = build_image_script(&project, /*is_bundle=*/ true);
        assert!(bundle.contains("tar -cf - --exclude=.iii ."), "{bundle}");
        assert!(!bundle.contains("--exclude=node_modules"));
    }

    /// The run of a build never installs; a local worker's code still syncs
    /// in, a bundle's is in the image.
    #[test]
    fn the_run_of_a_build_never_installs() {
        let project = project_with("apt-get install -y git", "npm ci");
        let local = build_image_run_script(&project, /*is_bundle=*/ false);
        assert!(
            !local.contains("apt-get") && !local.contains("npm ci"),
            "{local}"
        );
        assert!(!local.contains(".iii-prepared"));
        assert!(local.contains(RUN_CLEANUP.as_str()));
        assert!(local.contains(SOURCE_EXCLUDES.as_str()));
        assert!(local.contains("touch /run/iii/source-ready"));
        assert!(local.ends_with("exec /bin/sh -c 'node server.js'"));
        let bundle = build_image_run_script(&project, /*is_bundle=*/ true);
        assert!(
            !bundle.contains("npm ci") && !bundle.contains("mountpoint"),
            "{bundle}"
        );
        assert!(bundle.contains("cd /workspace"));
        assert!(bundle.ends_with("exec /bin/sh -c 'node server.js'"));
    }

    /// `.build` and `.workers` hold Compose's images and package copies: for
    /// `path://.` they are in the worker's dir, and stay out of the copy at
    /// any depth, as the dependency dirs do.
    #[test]
    fn compose_dirs_stay_out_of_the_copy() {
        let temp = tempfile::tempdir().unwrap();
        let t = temp.path();
        put(
            t,
            &[
                "src/app.js",
                "src/.build/w/image.erofs",
                "src/.workers/pkg/index.js",
                "src/node_modules/dep/index.js",
                "src/.iii/token",
                "src/sub/.build/x",
                "src/sub/code.js",
            ],
        );
        boot(t);
        assert!(t.join("ws/app.js").is_file() && t.join("ws/sub/code.js").is_file());
        for gone in [".build", ".workers", "node_modules", ".iii", "sub/.build"] {
            assert!(!t.join("ws").join(gone).exists(), "{gone} was copied");
        }
    }

    /// One list: what the run's copy leaves out is what the source sync
    /// ignores; the build's copy leaves out the same but `vendor`. Build-output
    /// names that also name source count as a directory at the root only.
    #[test]
    fn one_list_drives_the_copy_and_the_sync() {
        use super::super::source_watcher::should_ignore_path;
        let excludes: Vec<&str> = SOURCE_EXCLUDES.split(' ').collect();
        let build: Vec<&str> = BUILD_SOURCE_EXCLUDES.split(' ').collect();
        let synced = |rel: &str| !should_ignore_path(&Path::new("/p").join(rel), Path::new("/p"));
        for name in IGNORED_DIR_NAMES {
            let flag = format!("--exclude={name}");
            assert!(excludes.contains(&flag.as_str()), "{name} is copied");
            assert_eq!(
                build.contains(&flag.as_str()),
                !BUILD_COPIES.contains(name),
                "{name} in the build's copy"
            );
            assert!(!synced(&format!("sub/{name}/x")), "{name} syncs");
        }
        let loop_names = format!("for name in {}; do", ROOT_IGNORED_DIR_NAMES.join(" "));
        for copy in [&*RUN_COPY, &*INSTALL_COPY, &*BUILD_COPY] {
            assert!(copy.contains(&loop_names), "{copy}");
        }
        for name in ROOT_IGNORED_DIR_NAMES {
            assert!(!excludes.iter().any(|f| f.ends_with(&format!("={name}"))));
            assert!(!synced(&format!("{name}/x")) && synced(&format!("sub/{name}/x")));
        }
        for name in ["target", "dist"] {
            assert!(IGNORED_DIR_NAMES.contains(&name), "{name} at any depth");
        }
        assert_eq!(excludes.len(), IGNORED_DIR_NAMES.len() + 1);
        assert_eq!(
            build.len(),
            IGNORED_DIR_NAMES.len() + 1 - BUILD_COPIES.len()
        );
        for flags in [&excludes, &build] {
            assert!(flags.contains(&"--exclude=.iii"));
        }
        // The cleanup reads the manifests the copies write.
        assert!(RUN_COPY.contains(RUN_MANIFEST) && BUILD_COPY.contains(BUILD_MANIFEST));
        for cleanup in [&*RUN_CLEANUP, &*INSTALL_CLEANUP] {
            for manifest in [RUN_MANIFEST, BUILD_MANIFEST] {
                assert!(cleanup.contains(&format!("$IIISTATE/{manifest}")));
            }
            // It skips what the copy it runs before leaves out.
            for name in [".iii", "node_modules", "target"] {
                assert!(cleanup.contains(&format!("*/\"{name}\"/*")), "{name}");
            }
        }
        assert!(RUN_CLEANUP.contains(r#"*/"vendor"/*"#));
        assert!(!INSTALL_CLEANUP.contains(r#"*/"vendor"/*"#));
    }

    /// `build`, `out`, `obj`, `coverage` and `deps` often hold source below the
    /// root, and a root `build` is often a script: they are copied and synced
    /// there. Only a root directory of that name is build output, left out.
    #[test]
    fn root_only_names_match_a_root_directory_only() {
        use super::super::source_watcher::should_ignore_path;
        let temp = tempfile::tempdir().unwrap();
        let t = temp.path();
        put(
            t,
            &[
                "src/src/pkg/build/x.py",
                "src/src/out/y.ts",
                "src/lib/deps/z.go",
                "src/out/artifact.bin",
                "src/obj/Debug/a.dll",
                "src/node_modules/m.js",
                "src/pkg/target/x",
            ],
        );
        std::fs::write(t.join("src/build"), "#!/bin/sh\necho building\n").unwrap();
        for copy in [&*RUN_COPY, &*BUILD_COPY] {
            let _ = std::fs::remove_dir_all(t.join("ws"));
            std::fs::create_dir_all(t.join("ws")).unwrap();
            let (ok, log) = fragment(t, copy);
            assert!(ok, "{log}");
            for reached in [
                "build",
                "src/pkg/build/x.py",
                "src/out/y.ts",
                "lib/deps/z.go",
            ] {
                assert!(
                    t.join("ws").join(reached).is_file(),
                    "{reached} did not reach the guest"
                );
            }
            for left in ["out", "obj", "node_modules", "pkg/target"] {
                assert!(
                    !t.join("ws").join(left).exists(),
                    "{left} reached the guest"
                );
            }
        }
        let src = t.join("src");
        assert!(!should_ignore_path(&src.join("src/pkg/build/x.py"), &src));
        assert!(
            !should_ignore_path(&src.join("build"), &src),
            "the root script"
        );
        assert!(should_ignore_path(&src.join("out/artifact.bin"), &src));
        assert!(should_ignore_path(&src.join("out"), &src));
        // A root `build/` the build left stays; a nested one is code.
        std::fs::remove_file(t.join("src/build")).unwrap();
        put(t, &["src/build/gen.txt"]);
        put(&t.join("ws"), &["src/pkg/build/stale.py"]);
        std::fs::remove_file(t.join("ws/build")).unwrap();
        put(&t.join("ws"), &["build/from-the-image"]);
        boot(t);
        assert!(t.join("ws/build/from-the-image").is_file());
        assert!(
            !t.join("ws/build/gen.txt").exists(),
            "the host's root build/ was copied"
        );
        // Written by the worker, so it stays, as any file the worker writes.
        assert!(t.join("ws/src/pkg/build/stale.py").is_file());
    }

    /// A start without a build installs in the guest, after its copy: that
    /// copy is a build's (`vendor/` included). A run of an image is not.
    #[test]
    fn a_start_that_installs_copies_as_a_build_does() {
        let project = project_with("", "go build ./...");
        for overlay in [true, false] {
            let script = build_libkrun_local_script(&project, false, false, overlay);
            assert!(script.contains(BUILD_SOURCE_EXCLUDES.as_str()), "{script}");
            assert!(!script.contains("--exclude=vendor "), "{script}");
        }
        let image = build_image_run_script(&project, false);
        assert!(image.contains(SOURCE_EXCLUDES.as_str()));
        assert!(image.contains("--exclude=vendor "));
    }

    /// A run never installs, so what the build put in a dependency or build
    /// dir must survive every boot, whatever the host has there.
    #[test]
    fn every_dependency_dir_the_build_filled_survives_the_boots() {
        let temp = tempfile::tempdir().unwrap();
        let t = temp.path();
        put(t, &["src/main.py", "src/svc/app.py"]);
        std::fs::create_dir_all(t.join("ws")).unwrap();
        let (ok, log) = fragment(t, &BUILD_COPY);
        assert!(ok, "{log}");
        let built = [
            "venv/bin/python",
            "vendor/pkg/a.php",
            ".bundle/config",
            "deps/phoenix/mix.exs",
            "_build/dev/app.beam",
            "svc/.tox/py/x",
            "svc/dist/app.js",
            "svc/target/app.jar",
        ];
        put(&t.join("ws"), &built);
        // The host's own copies of them never reach the guest.
        put(
            t,
            &[
                "src/venv/bin/python",
                "src/svc/dist/app.js",
                "src/svc/target/app.jar",
            ],
        );
        for _ in 0..2 {
            boot(t);
            for file in built {
                assert!(t.join("ws").join(file).is_file(), "{file} was removed");
                assert_eq!(
                    std::fs::read_to_string(t.join("ws").join(file)).unwrap(),
                    "x"
                );
            }
        }
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
        // Compose's state holds the project's token.
        assert!(script.contains("--exclude=.iii "));
        // The guest receives real file operations over iii.source-sync.
        assert!(!script.contains("CHOKIDAR_USEPOLLING"));
        assert!(!script.contains("WATCHFILES_FORCE_POLLING"));
        assert!(!script.contains("TSC_WATCHFILE"));
    }

    /// Run a script fragment of the copy or the cleanup with `$SRC`, `$WS`
    /// and `$IIISTATE` pointing at `t`'s `src`, `ws` and `state`, under bash
    /// (the host's `sh` may lack `pipefail`, which the guest's has).
    fn fragment(t: &Path, script: &str) -> (bool, String) {
        // The guest's `$WS` is canonical, and the cleanup checks it is.
        let t = &std::fs::canonicalize(t).unwrap();
        let out = std::process::Command::new("bash")
            .arg("-c")
            .arg(format!("set -e\n{script}"))
            .env("SRC", t.join("src"))
            .env("WS", t.join("ws"))
            .env("IIISTATE", t.join("state"))
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn put(root: &Path, files: &[&str]) {
        for file in files {
            std::fs::create_dir_all(root.join(file).parent().unwrap()).unwrap();
            std::fs::write(root.join(file), "x").unwrap();
        }
    }

    /// A boot of a `path://` run: the cleanup, then the run's copy.
    fn boot(t: &Path) -> String {
        std::fs::create_dir_all(t.join("ws")).unwrap();
        let (ok, log) = fragment(t, &format!("{}\n{}", *RUN_CLEANUP, *RUN_COPY));
        assert!(ok, "{log}");
        log
    }

    /// A copy an older adapter made into a persistent /workspace must not
    /// survive the next boot: the token in it stays valid.
    #[test]
    fn workspace_cleanup_removes_a_stale_compose_state_copy() {
        let temp = tempfile::tempdir().unwrap();
        let t = temp.path();
        put(
            t,
            &["ws/.iii/compose/default/state.json", "ws/sub/.iii/token"],
        );
        put(t, &["src/.iii/compose/default/state.json", "src/app.js"]);
        // A disk an older adapter wrote has no manifest: nothing tells what
        // the host deleted before, and the boot says it stays.
        let log = boot(t);
        assert!(
            log.contains("no workspace manifest yet")
                && log.contains("stays in")
                && log.contains("until the worker is destroyed"),
            "{log}"
        );
        assert!(!t.join("ws/.iii").exists() && !t.join("ws/sub/.iii").exists());
        assert!(t.join("ws/app.js").is_file());
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

    /// A valid base image without bash (`node:20-alpine`) must still run the
    /// script, through the image's own `sh`.
    #[test]
    fn run_script_command_falls_back_to_sh_without_bash() {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        std::os::unix::fs::symlink("/bin/sh", bin.join("sh")).unwrap();
        let script = temp.path().join("dev-run.sh");
        std::fs::write(&script, "exit 7\n").unwrap();

        let status = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(RUN_SCRIPT_CMD.replace("/opt/iii/dev-run.sh", script.to_str().unwrap()))
            .env("PATH", &bin)
            .status()
            .unwrap();

        assert_eq!(status.code(), Some(7));
    }

    /// The VM env already holds the manifest env with the caller's overrides
    /// and the gateway rewrite applied; re-exporting the raw manifest values
    /// in the script would revert both.
    #[test]
    fn build_libkrun_local_script_does_not_reexport_manifest_env() {
        let project = ProjectInfo {
            name: "test".to_string(),
            kind: Some("typescript".to_string()),
            setup_cmd: String::new(),
            install_cmd: String::new(),
            run_cmd: "node server.js".to_string(),
            env: HashMap::from([("API_URL".to_string(), "http://localhost:3000".to_string())]),
            base_image: None,
        };
        let script = build_libkrun_local_script(
            &project, /*prepared=*/ true, /*is_bundle=*/ false, false,
        );
        assert!(!script.contains("API_URL"), "{script}");
        assert!(script.ends_with("exec /bin/sh -c 'node server.js'"));
    }

    #[test]
    fn current_prepared_marker_matches_copy_in_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join(".iii-prepared");
        std::fs::write(&marker, PREPARED_MARKER_VERSION).unwrap();

        assert!(prepared_marker_matches_current_workspace(&marker));
    }

    /// The cleanup removes what an earlier copy put there and the host no
    /// longer has, and nothing else: not what the build installed, at any
    /// depth and under any name, nor what the worker wrote. A directory that
    /// still holds such things stays; one the host turned into a file is moved
    /// aside so the copy can put the file there.
    #[test]
    fn the_cleanup_removes_only_what_the_host_deleted() {
        let temp = tempfile::tempdir().unwrap();
        let t = temp.path();
        put(
            t,
            &[
                "src/app.js",
                "src/gone.js",
                "src/old/a.js",
                "src/service/main.js",
                "src/swapped/x.js",
                "src/flip",
            ],
        );
        // A first boot has an empty /workspace: nothing stale to tell about.
        assert!(!boot(t).contains("no workspace manifest yet"));
        // What the build or the guest installed, and what the worker wrote.
        put(
            t,
            &[
                "ws/packages/a/dist/index.js",
                "ws/tools/cli/target/release/cli",
                "ws/service/node_modules/dep/index.js",
                "ws/old/node_modules/x.js",
                "ws/swapped/node_modules/y.js",
                "ws/data.db",
            ],
        );
        // The host deletes, and turns a dir into a file and a file into a dir.
        for gone in ["src/gone.js", "src/old/a.js"] {
            std::fs::remove_file(t.join(gone)).unwrap();
        }
        std::fs::remove_dir(t.join("src/old")).unwrap();
        std::fs::remove_dir_all(t.join("src/swapped")).unwrap();
        std::fs::write(t.join("src/swapped"), "now a file").unwrap();
        std::fs::remove_file(t.join("src/flip")).unwrap();
        put(t, &["src/flip/inner.js"]);
        // An earlier boot already moved a `swapped` aside.
        std::fs::write(t.join("ws/swapped.iii-kept.1"), "earlier").unwrap();

        let log = boot(t);
        for kept in [
            "packages/a/dist/index.js",
            "tools/cli/target/release/cli",
            "service/node_modules/dep/index.js",
            "old/node_modules/x.js",
            "data.db",
            "app.js",
            "service/main.js",
            "flip/inner.js",
        ] {
            assert!(t.join("ws").join(kept).is_file(), "{kept} is gone");
        }
        assert!(!t.join("ws/gone.js").exists() && !t.join("ws/old/a.js").exists());
        assert_eq!(
            std::fs::read_to_string(t.join("ws/swapped")).unwrap(),
            "now a file"
        );
        // The next free name: the earlier one stays as it was.
        assert!(
            t.join("ws/swapped.iii-kept.2/node_modules/y.js").is_file(),
            "{log}"
        );
        assert_eq!(
            std::fs::read_to_string(t.join("ws/swapped.iii-kept.1")).unwrap(),
            "earlier"
        );
        assert!(
            log.contains("swapped is no longer a directory on the host; what the build or the worker left in it is at swapped.iii-kept.2"),
            "{log}"
        );
        assert!(!log.contains("no workspace manifest yet"), "{log}");
    }

    /// A build's manifest ships in its image: the first run's cleanup removes
    /// what the build copied and the host deleted since.
    #[test]
    fn the_first_run_cleans_up_after_the_build_copy() {
        let temp = tempfile::tempdir().unwrap();
        let t = temp.path();
        put(t, &["src/app.js", "src/gone.js", "src/vendor/mod.go"]);
        std::fs::create_dir_all(t.join("ws")).unwrap();
        let (ok, log) = fragment(t, &BUILD_COPY);
        assert!(ok, "{log}");
        assert!(t.join("state").join(BUILD_MANIFEST).is_file());
        assert!(
            t.join("ws/vendor/mod.go").is_file(),
            "the build copies vendor/"
        );
        std::fs::remove_file(t.join("src/gone.js")).unwrap();
        let log = boot(t);
        assert!(!log.contains("no workspace manifest yet"), "{log}");
        assert!(!t.join("ws/gone.js").exists() && t.join("ws/app.js").is_file());
        // A run neither copies nor removes vendor/: the build's stays.
        assert!(t.join("ws/vendor/mod.go").is_file());
        assert!(t.join("state").join(RUN_MANIFEST).is_file());
        let manifest = std::fs::read_to_string(t.join("state").join(RUN_MANIFEST)).unwrap();
        assert!(manifest.lines().any(|l| l == "./app.js"), "{manifest}");
        assert!(!manifest.contains("vendor"), "{manifest}");
    }

    /// The cleanup never acts through a symlink: an entry whose parent in the
    /// guest is a link (the worker's, the install's, or one the sync applied)
    /// is skipped, so nothing outside `$WS` is removed, emptied or moved.
    #[test]
    fn the_cleanup_never_removes_through_a_symlinked_parent() {
        let temp = tempfile::tempdir().unwrap();
        let t = temp.path();
        put(
            t,
            &[
                "src/data/seed.json",
                "src/data/sub/a.js",
                "src/lib/deep/x.js",
            ],
        );
        boot(t);
        // The guest turns data/ and lib/ into links to a dir outside $WS.
        put(
            t,
            &[
                "victim/seed.json",
                "victim/sub/a.js",
                "victim/sub/extra.js",
                "victim/deep/x.js",
            ],
        );
        std::fs::remove_dir_all(t.join("ws/data")).unwrap();
        std::os::unix::fs::symlink(t.join("victim"), t.join("ws/data")).unwrap();
        std::fs::remove_dir_all(t.join("ws/lib")).unwrap();
        std::os::unix::fs::symlink("../victim", t.join("ws/lib")).unwrap();
        // The host deletes the files, and turns data/sub into a file.
        std::fs::remove_file(t.join("src/data/seed.json")).unwrap();
        std::fs::remove_dir_all(t.join("src/data/sub")).unwrap();
        std::fs::write(t.join("src/data/sub"), "now a file").unwrap();
        std::fs::remove_dir_all(t.join("src/lib/deep")).unwrap();

        let (ok, log) = fragment(t, &RUN_CLEANUP);
        assert!(ok, "{log}");
        for kept in ["seed.json", "sub/a.js", "sub/extra.js", "deep/x.js"] {
            assert!(t.join("victim").join(kept).is_file(), "{kept} went: {log}");
        }
        let mut victim: Vec<_> = std::fs::read_dir(t.join("victim"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        victim.sort();
        assert_eq!(victim, ["deep", "seed.json", "sub"], "{log}");

        // Nor through a `$WS` that is itself a link.
        let temp = tempfile::tempdir().unwrap();
        let t = temp.path();
        put(t, &["real/gone.js", "src/app.js"]);
        std::os::unix::fs::symlink("real", t.join("ws")).unwrap();
        std::fs::create_dir_all(t.join("state")).unwrap();
        std::fs::write(t.join("state").join(RUN_MANIFEST), "./\n./gone.js\n").unwrap();
        let (ok, log) = fragment(t, &RUN_CLEANUP);
        assert!(ok, "{log}");
        assert!(t.join("real/gone.js").is_file(), "{log}");
        assert!(log.contains("is not a real directory"), "{log}");
    }

    /// A run neither copies nor syncs `vendor/`, so it does not clean it
    /// either, though the build's manifest lists what the build copied there:
    /// the build's `vendor/` stays whole until the next build. A run that
    /// installs copies `vendor/` as a build does, and cleans it.
    #[test]
    fn a_run_leaves_the_vendor_dir_of_the_build_alone() {
        let temp = tempfile::tempdir().unwrap();
        let t = temp.path();
        put(
            t,
            &["src/app.js", "src/vendor/a/mod.go", "src/vendor/b/old.go"],
        );
        std::fs::create_dir_all(t.join("ws")).unwrap();
        let (ok, log) = fragment(t, &BUILD_COPY);
        assert!(ok, "{log}");
        let manifest = std::fs::read_to_string(t.join("state").join(BUILD_MANIFEST)).unwrap();
        assert!(
            manifest.lines().any(|l| l == "./vendor/b/old.go"),
            "{manifest}"
        );
        std::fs::remove_dir_all(t.join("src/vendor/b")).unwrap();

        let log = boot(t);
        assert!(t.join("ws/vendor/b/old.go").is_file(), "{log}");
        assert!(t.join("ws/vendor/a/mod.go").is_file());

        let (ok, log) = fragment(t, &format!("{}\n{}", *INSTALL_CLEANUP, *INSTALL_COPY));
        assert!(ok, "{log}");
        assert!(!t.join("ws/vendor/b").exists(), "{log}");
        assert!(t.join("ws/vendor/a/mod.go").is_file());
    }

    /// Only a line `tar -v` printed as one plain name counts. A name with a
    /// newline (busybox prints it raw, GNU tar escapes it) or a backslash is
    /// never cleaned, so no fragment or escape of it removes a worker's file.
    #[test]
    fn the_cleanup_trusts_plain_manifest_lines_only() {
        let temp = tempfile::tempdir().unwrap();
        let t = temp.path();
        put(
            t,
            &[
                "src/app.js",
                "ws/app.js",
                "ws/gone.js",
                "ws/notes",
                "ws/frag",
                "ws/a\\nb",
                "ws/back\\slash",
                "ws/tail",
            ],
        );
        std::fs::create_dir_all(t.join("state")).unwrap();
        std::fs::write(
            t.join("state").join(RUN_MANIFEST),
            "./\n./app.js\n./gone.js\n./notes\nfrag\n./a\\nb\n./back\\slash\n./tail\nmore\n",
        )
        .unwrap();
        let (ok, log) = fragment(t, &RUN_CLEANUP);
        assert!(ok, "{log}");
        assert!(!t.join("ws/gone.js").exists(), "the cleanup ran: {log}");
        for kept in ["app.js", "notes", "frag", "a\\nb", "back\\slash", "tail"] {
            assert!(t.join("ws").join(kept).is_file(), "{kept} went: {log}");
        }
    }

    /// In legacy mode the marker sits on the guest root: a planted symlink
    /// must not be followed and a FIFO must not block the daemon.
    #[test]
    fn prepared_marker_refuses_symlinks_and_fifos() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        std::fs::write(&real, PREPARED_MARKER_VERSION).unwrap();
        let marker = temp.path().join(".iii-prepared");
        std::os::unix::fs::symlink(&real, &marker).unwrap();
        assert!(!prepared_marker_matches_current_workspace(&marker));

        std::fs::remove_file(&marker).unwrap();
        nix::unistd::mkfifo(&marker, nix::sys::stat::Mode::S_IRWXU).unwrap();
        assert!(!prepared_marker_matches_current_workspace(&marker));
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
        // Re-sync deletions: remove what the last copy put there and the host
        // deleted, so it doesn't keep running on the persistent upper.
        assert!(script.contains(INSTALL_CLEANUP.as_str()));
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
            image: None,
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
