# Changelog

## 0.4.1

Sandbox contract 1.4: binary packages in VMs, a build's `base_image`, and a
`worker::destroy` that keeps the VM's data unless asked to `prune`. The S-codes
are those of 0.4.0.

### Added

- **Binary packages in VMs.** A `Binary` source names the directory where
  Compose put a binary package's Linux build and its `executable`.
  `worker::build` needs a `base_image` for it (S001 without one), copies the
  package into the image and runs its `scripts.install` when it ships a
  manifest; a start of the build runs the executable from `/workspace`. A
  binary package boots only a build (S501 on `worker::prepare`, or a start
  without `build`). `adapter::info` advertises `worker.binary` in the overlay
  mode.
- **A build's `base_image`.** `worker::build` takes a `base_image` that overrides
  the manifest's `runtime.base_image` for every source; a local worker's
  manifest may then leave it out.
- **`worker::destroy` keeps the data.** Without `prune`, a destroy removes the
  VM's runtime state and its build-dir record, which frees its build directory,
  and keeps its data disk, the pre-build copies of an earlier adapter and the
  seal: a later start of the same `state_key` boots over the same data. A VM
  with no data goes whole; `destroyed` is false when only data was there.
  `prune: true` removes everything, as 0.4.0 did. `adapter::info` advertises
  `worker.prune`.

### Fixed

- **A build publishes its image without a stale id.** `worker::build` flushes
  the new image to disk, removes `<output_dir>/image.id`, renames the new
  `image.erofs` into place and renames the new `image.id` last, flushing the
  directory between the steps. A crash, a failed flush or a failed rename
  between the two renames leaves an image without an id, which `worker::start`
  refuses with S508 until the next build. 0.4.0 renamed the image beside the
  previous build's id, and a start accepted that pair. A filesystem that cannot
  flush a directory gets a warning, as in 0.4.0, not a failed build.
- **A scoped token no longer writes or boots another VM's build directory.**
  A VM's build directory is the last `output_dir` (`worker::build`) or
  `build_dir` (`worker::start`) its scoped token presented, recorded in
  `~/.iii/sandbox/workers/<vm_id>/build-dir` with a MAC of the project token.
  On a directory another VM of the same project token recorded, a scoped token
  gets S505 before anything is created or written there: the token Compose
  hands container A can no longer replace, or boot, the `image.erofs` and
  `image.id` in a directory container B's VM recorded. The message names B's
  record and the way out (destroy that VM, or delete that file). Presenting
  another directory moves the VM there and releases the old one; a second
  spelling of a recorded directory (same device and inode) is still its VM's;
  `worker::destroy` deletes the VM's record; the project token is not checked.
  The check reads no directory name.

### Upgrading from 0.4.0

Each VM records its build directory on its next scoped build or start. A caller
that wants 0.4.0's `worker::destroy`, which removed the data disk too, sends
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
