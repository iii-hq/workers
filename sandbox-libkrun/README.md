# sandbox-libkrun

The libkrun sandbox adapter: it runs sandboxes and worker VMs as libkrun
microVMs on the engine's host. The engine's `sandbox::*` router forwards each
sandbox call to it as `sandbox-libkrun::<operation>`, and Compose boots
`package://` and `path://` VM workers through its `sandbox-libkrun::worker::*`
functions. You rarely call it directly: callers use `sandbox::*`, and Compose
installs this adapter when a project needs it.

## Install

Compose adds it on its own when a project needs sandboxes or VM workers. To
add it by hand on iii 0.26 or later, use the `iii-sandbox` alias, which pins
the adapter to the sandbox contract major the engine supports:

```bash
iii call compose::add worker=iii-sandbox
```

Older Compose has no such alias, so pin the range yourself (0.2.x implements
sandbox contract 1). On iii 0.25 and earlier the command is `iii trigger`
(iii 0.26 accepts it as an alias of `iii call`):

```bash
iii trigger compose::add worker=sandbox-libkrun@^0.2
```

Calling `sandbox::*` through the router needs iii 0.26 or later; the
`sandbox-libkrun::*` ids work on iii 0.24 and later.

## Quickstart

Run code in a fresh VM through the router, which forwards to this adapter:

```bash
iii call sandbox::run --timeout-ms 300000 \
  --json '{"image":"python","code":"print(6*7)","lang":"python"}'
```

The reply carries `stdout` (`42`), `stderr` and `exit_code`; the VM stops
when the call returns. The first call pulls the image, so it is slower. `sandbox-libkrun::run` takes the same payload directly.

## What it serves

| Functions | Purpose |
|---|---|
| `create`, `exec`, `stop`, `list`, `catalog::list`, `run` | Sandbox lifecycle and one-call runs |
| `fs::ls`, `fs::stat`, `fs::mkdir`, `fs::rm`, `fs::chmod`, `fs::mv`, `fs::grep`, `fs::sed`, `fs::write`, `fs::read` | Files inside a live sandbox |
| `adapter::info` | Adapter name, version and the sandbox contract version it implements |
| `worker::build`, `worker::prepare`, `worker::start`, `worker::status`, `worker::logs`, `worker::stop`, `worker::destroy` | Compose's worker VMs (see below) |

Every id is prefixed with `sandbox-libkrun::`.

## Configuration

The adapter reads its configuration entry once at startup with
`configuration::get` (`III_CONFIG_NAME`, else `sandbox-libkrun`). A change
applies when the adapter restarts. A config that does not parse, or a failed
read, stops the adapter with exit code 2 instead of falling back to defaults.

```yaml
auto_install: true              # pull a missing image on first create
image_allowlist: [python, node] # bootable images; empty denies every create
default_idle_timeout_secs: 300  # idle sandboxes are reaped after this
max_concurrent_sandboxes: 32
max_concurrent_exec_per_sandbox: 4  # 0 means 1, not unlimited
max_exec_timeout_ms: 300000     # ceiling on any exec's timeout_ms
default_cpus: 1
default_memory_mb: 512
per_image_caps:                 # optional per-image ceilings
  python: { max_cpus: 4, max_memory_mb: 2048 }
custom_images:                  # extra images: name -> OCI ref
  my-app: ghcr.io/acme/my-app:1.2.3   # must also be in image_allowlist
```

A `custom_images` key may not reuse a preset name (`python`, `node`), and the
old wrapped shape (`sandbox: {...}`) is rejected.

VM tuning stays in four environment variables the adapter passes to each
worker VM (sandboxes ignore them):

| Variable | Effect |
|---|---|
| `III_VM_HYPERTHREADING=1` | Enable SMT (hyperthreading) in the guest |
| `III_VM_NESTED_VIRT=1` | Enable nested virtualization |
| `III_VM_VIRTIOFS_SHM_SIZE_MIB=<n>` | virtiofs shared-memory window; `0` skips it |
| `III_VM_NOFILE_LIMIT=<n>` | Guest open-file limit; `0` leaves it to the guest init |

## Host prerequisites

- **macOS, Apple Silicon only.** The released binary is ad-hoc signed with
  the `com.apple.security.hypervisor` and
  `com.apple.security.cs.disable-library-validation` entitlements. An
  unsigned dev build signs itself in place on first run.
- **Linux, glibc.** The Compose user needs read/write access to `/dev/kvm`.
- Any other host gets a clear `UnsupportedPlatform` error from Compose when a
  project needs the adapter.

## Worker VMs

Compose runs VM workers through `sandbox-libkrun::worker::*`. Because those
requests name host directories (`host_path`) and carry the guest's
environment, two guards apply:

- **Token.** Compose generates a per-project token and passes it to the
  adapter as `III_SANDBOX_WORKER_TOKEN`. Every `worker::*` request carries it
  in `auth`; a mismatch gets S505.
- **Host roots.** `host_path` is accepted only under the project's Compose
  directory and the Compose package cache. Anything else gets S501 naming the
  two roots.

Compose also sets `III_DISABLE_TRACE_PAYLOADS=1` on the adapter, and every
`worker::*` function is registered `observable: false`, so at the engine's default
log level the token and guest env reach neither its trace store nor an observer.
At `debug` that holds only on an engine that keeps the payload out of its debug
log: an older one logs each request with its `auth`, which reaches the trace store
and any observer of `observability::on-log`.

The project token stays out of the guest, except in the cases under Known limits.
It lives in Compose's state (`.iii`), which the copy into `/workspace` and source
sync skip in any case, and the source share answers ENOENT for that name at any depth
and in any case, including to guest root mounting the share again after the copy.

`iii sandbox run` presents a token scoped to its VM instead of the project token
(HMAC-SHA256 keyed by the project token over `iii-sandbox/worker-scope/v1\0`
followed by the VM's `state_key`, in lowercase hex), which gets S505 on any other VM. A VM started with a scoped token also has a lease: when its last
log follower is gone and no client reattaches within 45 s, the adapter stops it
(SIGTERM, then SIGKILL after 10 s). VMs started with the project token have no
lease, because Compose supervises them itself.

### Builds

From 0.4.0 (sandbox contract 1.3) the adapter builds a VM's image and boots it,
so a run never installs anything. It advertises `worker.build` in
`adapter::info` only in the overlay rootfs mode; the legacy mode
(`III_ROOTFS_MODE=legacy`, or a build without an embedded iii-init) cannot boot
a build and keeps quiet about it.

- **`worker::build`** boots a build VM on the cached base image (pulling it when
  missing) with a fresh upper, copies the worker's code into `/workspace` (a
  bundle's whole package; a `path://` worker's code without the dependency dirs,
  `.build/`, `.workers/` and `.iii`), and runs `scripts.setup` and
  `scripts.install` there with the request's `env` only. When the script exits
  0, iii-init writes the merged root, without `/proc`, `/sys`, `/dev`, `/run`,
  `/tmp`, `/opt/iii` and `/mnt/host-src`, and the host turns it into
  `<output_dir>/image.erofs`, then `image.id`, each through a temporary file
  and a rename. The build streams its output into `logs` and writes a
  one-byte binary frame there at least every 5 s; when the channel closes or a
  write fails, it stops the build VM, deletes the temporary image, keeps the
  previous pair and frees the key. One build per `state_key` runs at a time
  (S509), and a failed step answers S507 with the output's last lines.
  `worker::destroy` of a VM whose build runs answers S509 too: cancel the build
  (close its `logs`) first.
- **`worker::start` with `build`** checks that `build_dir/image.id` names
  `build_id` (S508 otherwise, or when it is missing) and boots `image.erofs` as
  the overlay's lower, without setup or install. A `path://` worker's code
  still syncs in live; a bundle's is in the image.
- **Data disk.** A VM that boots builds keeps its writes on
  `~/.iii/sandbox/workers/<vm_id>/upper.ext4`, beside the guest's share
  `root/`: they outlive restarts and rebuilds, and only `worker::destroy`
  removes them. The overlay is mounted without `index`, `metacopy`,
  `redirect_dir` and `xino`, which lets a rebuild swap the lower under it.
- **Earlier disks.** The first start of a build writes the seal
  `<vm_id>/build-mode`. Without it, a 0.3 overlay disk (`root/upper.ext4`)
  moves to `upper.ext4.pre-build`, and a legacy rootfs clone (`root/`) to
  `root.pre-build/` with a fresh `root/`; the VM's log names the kept path.
  `worker::destroy` removes them with the rest.
- **What a `path://` copy leaves out.** One list, the source watcher's ignore
  list plus `.iii`: a run neither copies nor syncs those names, so what the
  build put there is never overwritten by the host's.
  - Dependency, cache, build-output, VCS and editor dirs match at any depth:
    among them `node_modules`, `.venv`, `venv`, `vendor`, `target`, `dist`,
    `__pycache__`, `.tox`, `.gradle`, `.git`, and Compose's `.build` and
    `.workers`.
  - `build`, `out`, `obj`, `coverage` and `deps`, which projects also use for
    source below the root, match only as a directory at the worker's root:
    `src/pkg/build/` is code and syncs, and so does a root `build` script.
  - A build leaves the same names out of its copy, except `vendor/`, where Go
    keeps vendored source. A `worker::start` without `build`, which still
    installs in the guest, copies as a build does.
- **What a boot removes from `/workspace`.** Each copy records what it
  extracted in a manifest outside `/workspace`, on the data disk
  (`/var/lib/iii/workspace.manifest`; a build's copy writes
  `/var/lib/iii/build.manifest` into its image). Before its copy, a boot
  removes only the entries of those manifests that the host no longer has, or
  has as another kind, deepest first. What the build installed and what the
  worker wrote in `/workspace` are in no manifest, so they stay across
  restarts and rebuilds, whatever their name or depth.
  - A boot never cleans a name its copy leaves out: a run leaves the
    `vendor/` the build copied whole until the next build, even where the
    host deleted files from it.
  - Nothing is removed through a symlink. An entry whose parent in the guest
    is not a chain of real directories inside `/workspace` (the worker, the
    install or the sync put a link there) is skipped.
  - A directory goes only once empty. When the host turned a directory into a
    file and the guest's directory still holds such files, that directory
    moves aside to the first free `<name>.iii-kept.<n>` (`.iii-kept.1`,
    `.iii-kept.2`, ...).
  - A name with a newline or a backslash is never cleaned: `tar -v` prints
    it escaped (GNU) or over several lines (busybox), and the cleanup trusts
    a line only when it is one plain name.
  - Without a manifest (a disk an older adapter wrote) nothing tells what
    the host deleted before: it stays in `/workspace` until the worker is
    destroyed, and the boot logs it. From that boot on, the copies record
    their manifest.
  - A file the host deletes while the VM runs stays in the guest until the
    next boot when it is a root `build`, `out`, `obj`, `coverage` or `deps`
    file.

## Known limits

The guards assume bus clients are trusted. A client that registers the
adapter's function ids or its worker name first, while the adapter is down,
receives the `auth` token and the guest `env` that Compose sends. The router
has the same limit for `sandbox::*` forwards.

Two races of a build are accepted:

- `worker::start` checks `image.id`, not the image: a rebuild that renames a new
  `image.erofs` between that check and the VM opening the file boots the newer
  image under the older id.
- The adapter notices a reader of `logs` that left only at its next write, up
  to 5 s later. A reader that leaves less than 5 s before a build ends can
  find the new pair installed: the build still renames it and replies with
  its build id. A caller that gave up (its `build.json` still names the older
  build) then gets S508 on its next start, and builds again.

The boot cleanup of `/workspace` trusts a manifest line only when it is one
plain name, but busybox tar (alpine images) prints names raw. A name that holds
a newline followed by `./` (`a<newline>./b`) prints as two lines that each look
like a valid entry, and the first (`./a`) can remove a file of that name that
the worker wrote in `/workspace` and the host does not have. GNU tar escapes such
names, so it never happens with it.

`.iii` is hidden by name, so `III_COMPOSE_STATE_DIR` pointing at another name
inside a worker's directory puts the token in that guest. The name hides the whole
directory, so a VM also misses anything else a project keeps there, such as
`.iii/project.ini`.

`data/` is not hidden. A managed engine keeps its trace store there, so a VM whose
directory contains it can read the traced payloads, and the token too when an older
engine logs at `debug`.

A VM that ran on an adapter before 0.3.1 with `.iii` in its directory keeps the
bytes of the token on its disk: the boot deletes the copy but does not scrub the
blocks. Compose keeps the token across `compose::update` and `compose::restart`,
so stop Compose and run `iii compose --up` after the upgrade, which replaces it.

## Errors

Errors carry an S-code with a `docs_url`. The full list is at
[iii.dev/docs/creating-workers/sandboxes](https://iii.dev/docs/creating-workers/sandboxes).
