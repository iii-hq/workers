# Firmware

`libkrunfw` is the guest kernel the microVMs boot, embedded into the binary
with `--features embed-libkrunfw`.

| | |
|---|---|
| libkrunfw version | 5.6.2 |
| ABI | 5 |
| Guest kernel | 6.12.109 |

| File | Platform |
|---|---|
| `libkrunfw-linux-x86_64.so` | Linux x86_64 |
| `libkrunfw-linux-aarch64.so` | Linux aarch64 |
| `libkrunfw-darwin-aarch64.dylib` | macOS Apple Silicon |

## Never LFS

These files are committed as plain git blobs. `.gitattributes` marks
`sandbox-libkrun/firmware/**` as `binary`. Do not put them behind Git LFS: a
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

Upstream is <https://github.com/libkrun/libkrunfw> (formerly
`containers/libkrunfw`), tag `v5.6.2`. The blobs come from that release's
assets; check each download against the sha256 GitHub lists for it.

- Linux: `libkrunfw-x86_64.tgz` and `libkrunfw-aarch64.tgz` each hold
  `lib/<arch>-linux-gnu/libkrunfw.so.5.6.2`. Copy that file over
  `libkrunfw-linux-<arch>.so`.
- macOS: `libkrunfw-prebuilt-aarch64.tgz` holds `libkrunfw/kernel.c` (the
  aarch64 guest kernel as a C array). On an Apple Silicon Mac, compile it
  the way its Makefile does, pinning the deployment target so older macOS
  loads it, and copy the result over `libkrunfw-darwin-aarch64.dylib`:

  ```sh
  /usr/bin/cc -mmacosx-version-min=14.0 -fPIC -DABI_VERSION=5 -shared \
    -Wl,-install_name,libkrunfw.5.dylib -o libkrunfw.5.dylib libkrunfw/kernel.c
  ```

Or build from source on the target platform following the upstream README
(`make`) and copy the shared library over the matching file here.

## Refreshing `SHA256SUMS`

After changing or adding any blob, regenerate the sums from this directory and
commit both together:

```sh
cd sandbox-libkrun/firmware
shasum -a 256 libkrunfw-* > SHA256SUMS
```

Then run `cargo test --locked --test firmware_check`.

## iii-init

The guest init binary is not stored here. Build it with
`scripts/build-init.sh` (see that script) and embed it with `--features embed-init`.
