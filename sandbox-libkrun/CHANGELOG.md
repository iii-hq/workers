# Changelog

## 0.4.1

Sandbox contract 1.3, unchanged: the wire types, the fixtures and the S-codes
are those of 0.4.0, so Compose's `^0.4` installs it as is.

### Fixed

- **A build publishes its image without a stale id.** `worker::build` removes
  `<output_dir>/image.id`, flushes that to disk, renames the new `image.erofs`
  into place and renames the new `image.id` last. A crash between the renames
  leaves an image without an id, which `worker::start` refuses with S508 until
  the next build. 0.4.0 renamed the image beside the previous build's id, and a
  start accepted that pair.
- **A scoped token writes and boots only its own VM's build directory.** The
  first `output_dir` (`worker::build`) or `build_dir` (`worker::start`) a VM's
  scoped token presents becomes that VM's build directory, recorded in
  `~/.iii/sandbox/workers/<vm_id>/build-dir` with a MAC of the project token.
  Another directory gets S505 while the recorded one exists, and so does one
  another VM recorded: the token Compose hands container A can no longer
  replace container B's `image.erofs` and `image.id`. A directory that is gone
  is replaced by the next one presented, `worker::destroy` forgets the record,
  and the project token is not checked. The check reads no directory name.
