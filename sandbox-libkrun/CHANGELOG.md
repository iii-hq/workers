# Changelog

## 0.5.0

Sandbox contract 1.4: binary packages in VMs, a build's `base_image`, and a
`worker::destroy` that keeps the VM's data unless asked to `prune`. The S-codes
are those of 0.4.x. 0.4.1 to 0.4.3 were published from `main` with contract
1.3; 0.5.0 includes them (0.4.3's `worker::logs` writer that connects at once).

### Added

- **Binary packages in VMs.** A `Binary` source names the directory where
  Compose put a binary package's Linux build and its `executable`.
  `worker::build` needs a `base_image` for it (S001 without one), copies the
  package into the image and runs its `scripts.install` when it ships a
  manifest; a start of the build runs the executable from `/workspace`. A
  binary package boots only a build (S501 on `worker::prepare`, or a start
  without `build`). Its `executable` is S501 with a `.` component or as
  `init.krun` at the package root, the guest init's name, which the build's
  copy leaves out; the build fails (S507) when its scripts leave the executable
  no executable regular file. `III_BUNDLE_WORKERS_DISABLED=1` refuses bundle
  sources only, not binary packages. `adapter::info` advertises
  `worker.binary` in the overlay mode.
- **A build's `base_image`.** `worker::build` takes a `base_image` that overrides
  the manifest's `runtime.base_image` for every source; a local worker's
  manifest may then leave it out.
- **`worker::destroy` keeps the data.** Without `prune`, a destroy removes the
  VM's runtime state and its build-dir record, which frees its build directory,
  keeps its data disk and the pre-build copies, and keeps or writes the seal; a
  VM with no data goes whole, seal included. It first moves the disk of a start
  without `build`, or a legacy rootfs clone, to the next free pre-build name,
  so it never deletes what a guest wrote. A later start with `build` of the
  same `state_key` boots on the kept data disk; a start without `build` begins
  with a new disk, and what the previous one held stays under the pre-build
  names. The reply lists what stayed in `kept`; `destroyed` is false when only
  data was there. `prune: true` removes everything, as 0.4.x did.
  `adapter::info` advertises `worker.prune` in the overlay mode, with
  `worker.build`; the legacy mode keeps the data the same way without
  advertising it, so a caller that wants the data gone sends `prune: true`.

### Fixed

- **A build publishes its image without a stale id.** `worker::build` flushes
  the new image to disk, removes `<output_dir>/image.id`, renames the new
  `image.erofs` into place and renames the new `image.id` last, flushing the
  directory between the steps. A crash, a failed flush or a failed rename
  between the two renames leaves an image without an id, which `worker::start`
  refuses with S508 until the next build. 0.4.x renamed the image beside the
  previous build's id, and a start accepted that pair. A filesystem that cannot
  flush a directory gets a warning, as in 0.4.x, not a failed build.
- **A scoped token no longer writes or boots another VM's build directory.**
  A VM's build directory is the last `output_dir` (`worker::build`) or
  `build_dir` (`worker::start`) its scoped token presented, recorded in
  `~/.iii/sandbox/workers/<vm_id>/build-dir` with a MAC of the project token.
  On a directory another VM of the same project token recorded, a scoped token
  gets S505 before anything is written there, and before anything is created
  there but a second spelling of a recorded directory that did not exist yet,
  which the build creates, empty, and then refuses: the token Compose
  hands container A can no longer replace, or boot, the `image.erofs` and
  `image.id` in a directory container B's VM recorded. The message names B's
  record and the way out (destroy that VM, or delete that file). Presenting
  another directory moves the VM there and releases the old one; a second
  spelling of a recorded directory (same device and inode) is still its VM's;
  `worker::destroy` deletes the VM's record; the project token is not checked.
  The check reads no directory name. A build creates its `output_dir` one
  component at a time through no symlink, so a link put in its path since the
  check is S501 and creates nothing outside the host roots.

### Release

Publish this release with the exact version `0.5.0`, never as a patch bump of
0.4.3: the bytes say 0.5.0 and contract 1.4, and a 0.4.x version would reach
Compose installs pinned to `^0.4` (contract 1.3), which send no `prune`, so
their `compose::remove` would keep the data. The caret range is the only guard.

### Upgrading from 0.4.x

Each VM records its build directory on its next scoped build or start. A caller
that wants 0.4.x's `worker::destroy`, which removed the data disk too, sends
`prune: true`; without it the data stays.

### Known limits

- A scoped token takes any directory that no VM of the same project token has
  recorded now: the directory of a destroyed VM whose build directory stayed,
  the new path after a move until its container builds or starts, every build
  directory of another Compose namespace (a container's build directory sits
  beside the compose file in every namespace, and records count only under
  their own token), and, on a filesystem that cannot flush a directory, a
  record a power failure lost.
- A VM whose `state_key` changes while its project token survives leaves its
  record behind: the same `state.json` through another spelling of
  `III_COMPOSE_STATE_DIR`, or state moved or copied with its token to another
  `III_COMPOSE_STATE_DIR`, while the compose file, and so the container's build
  directory, stays where it is. The new VM then gets S505 on its own build
  directory until the old VM is destroyed or its `build-dir` deleted, as the
  message says. Moving or copying the whole project changes the build
  directory too, and nothing gets S505; a new state directory brings a new
  token, and the old records stop counting.
- Where the filesystem cannot lock a directory (NFS), claims are serialized
  within one adapter process only, with a warning.
- Publishing assumes one writer per `output_dir` at a time; two namespaces
  building one container at once are not serialized.
