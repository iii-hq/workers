# curl_impersonate_sys

Repository-owned, minimal raw Rust bindings for the native engine frozen by
curl-cffi 0.16.0. The browser worker links it only when its
`scrapling-compat` feature is explicitly enabled; normal safe builds retain
the artifact-free reqwest engine.

## Frozen upstream

- curl-cffi [`v0.16.0`](https://github.com/lexiforest/curl_cffi/releases/tag/v0.16.0)
  says its packaged engine is curl 8.21 with curl-impersonate 2.0.
- Its tagged
  [`Makefile`](https://github.com/lexiforest/curl_cffi/blob/v0.16.0/Makefile)
  pins `VERSION := 2.0.0` and `CURL_VERSION := curl-8_21_0`.
- curl-impersonate [`v2.0.0`](https://github.com/lexiforest/curl-impersonate/releases/tag/v2.0.0)
  publishes official `libcurl-impersonate` GNU/Linux archives for both Tier-1
  architectures. The release states that curl was updated to 8.21.0.
- The easy-handle surface follows curl's official
  [libcurl easy interface](https://curl.se/libcurl/c/libcurl-easy.html). The
  extra `curl_easy_impersonate` declaration comes from the v2.0.0 patched
  `include/curl/easy.h` shipped in each verified archive.

`artifacts.manifest` records the immutable GitHub release asset URLs, byte
lengths, and GitHub-published SHA-256 digests:

| Rust target | Bytes | SHA-256 |
| --- | ---: | --- |
| `x86_64-unknown-linux-gnu` | 26,572,973 | `d8a98bc123fae4f04bb6a7584ff486333a334b3b08edaba1867929ae8d6ebb4d` |
| `aarch64-unknown-linux-gnu` | 25,595,976 | `12708019a6c1c3a7a7a40a8a379d12aaca127fcd13bda60b06fdb0e013f6433f` |

The archives contain a monolithic `libcurl-impersonate.a`, the matching curl
headers, and shared-library variants. They therefore provide the frozen
BoringSSL/nghttp2/ngtcp2/nghttp3-enabled build instead of relying on ambient
system curl packages.

## Artifact workflow

Normal safe builds need no native artifact and do not link libcurl:

```sh
cargo test --manifest-path vendor/curl_impersonate_sys/Cargo.toml
```

Artifact acquisition is explicit and separate from Cargo:

```sh
scripts/fetch_curl_impersonate_artifacts.sh x86_64-unknown-linux-gnu
scripts/fetch_curl_impersonate_artifacts.sh --verify x86_64-unknown-linux-gnu
```

Set `CURL_IMPERSONATE_ARTIFACT_DIR` to use a shared CI/release cache. Its
layout is `<cache>/<target>/<archive>` plus `<cache>/<target>/root/`.

A certified build never downloads. `build.rs` requires a supported Tier-1
target, checks the archive byte length and SHA-256 again, checks the extracted
static library/header, then links the static archive. Missing, corrupt, or
unsupported inputs fail the build:

```sh
cargo test --manifest-path vendor/curl_impersonate_sys/Cargo.toml \
  --features certified
```

The FFI is deliberately raw and small: global/easy lifecycle, option/info
varargs, list headers, impersonation, transfer callbacks, and the socket-open
callback used by the safe-mode egress gate. Higher-level request, ownership,
callback-unwind, and session policy belong in the integrating crate.

## Licensing

The bindings are MIT licensed. curl-impersonate's upstream MIT license is
included as `UPSTREAM_LICENSE`. Release artifacts also aggregate curl and its
native dependencies; a binary redistribution must preserve all notices from
the corresponding upstream source release.
