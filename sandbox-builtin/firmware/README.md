# Firmware

`libkrunfw` is the guest kernel the microVMs boot, embedded into the binary
with `--features embed-libkrunfw`.

| | |
|---|---|
| libkrunfw version | 5.2.1 |
| ABI | 5 |
| Guest kernel | 6.12.68 |

| File | Platform |
|---|---|
| `libkrunfw-linux-x86_64.so` | Linux x86_64 |
| `libkrunfw-linux-aarch64.so` | Linux aarch64 |
| `libkrunfw-darwin-aarch64.dylib` | macOS Apple Silicon |

## Never LFS

These files are committed as plain git blobs. `.gitattributes` marks
`sandbox-builtin/firmware/**` as `binary`. Do not put them behind Git LFS: a
checkout without the LFS filter yields a ~130 byte pointer file, which `build.rs`
rejects (it is not an ELF or Mach-O file).

## Verification

`build.rs` checks the file for the target when `embed-libkrunfw` is on, and
panics unless:

- a `.so` starts with the ELF magic, a `.dylib` with the 64-bit Mach-O magic,
- its sha256 equals its line in `SHA256SUMS`.

Without the feature, `build.rs` embeds a 1-byte placeholder and checks nothing.
The same check runs in `tests/firmware_check.rs`.

## Rebuilding a blob

Upstream is <https://github.com/containers/libkrunfw>, tag `v5.2.1`.

1. Build on the target platform (the Linux blobs on Linux, the dylib on macOS)
   following the upstream README (`make`).
2. Copy the resulting shared library over the matching file here, keeping the
   name from the table above.

## Refreshing `SHA256SUMS`

After changing or adding any blob, regenerate the sums from this directory and
commit both together:

```sh
cd sandbox-builtin/firmware
shasum -a 256 libkrunfw-* > SHA256SUMS
```

Then run `cargo test --locked --test firmware_check`.

## iii-init

The guest init binary is not stored here. Build it with
`scripts/build-init.sh` (see that script) and embed it with `--features embed-init`.
