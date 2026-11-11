# Changelog

## 0.4.1

Sandbox contract 1.3, unchanged: the wire types, the fixtures and the S-codes
are those of 0.4.0, so Compose's `^0.4` installs it as is.

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

Nothing to do: each VM records its directory on its next scoped build or
start.

### Known limits

- A scoped token takes any directory that no VM of the same project token has
  recorded now: the directory of a destroyed VM whose build directory stayed,
  the new path after a move until its container builds or starts, every
  directory of another Compose namespace (namespaces share
  `.iii/build/<container>`, and records count only under their own token), and
  a record a power failure lost.
- A VM whose `state_key` changes leaves its record behind, and the new VM gets
  S505 on the same directory until the old one is destroyed or its `build-dir`
  deleted.
- Publishing assumes one writer per `output_dir` at a time; two namespaces
  building one container at once are not serialized.
