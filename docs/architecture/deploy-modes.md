# Release artifact modes

The pinned iii compiler translates one root-catalog worker into an immutable
package descriptor and independent build units. The release train consumes
only that descriptor after prepare.

| `artifact.kind` | Prepared artifact | Publication |
|---|---|---|
| `rust-binary` | one deterministic `tar.gz` and checksum per target | GitHub Release URLs and digests keyed by Rust target |
| `javascript-bundle` | one deterministic archive from explicit files | GitHub Release archive URL and digest |
| `python-bundle` | one deterministic archive from explicit files | GitHub Release archive URL and digest |
| `oci-image` | deterministic OCI-layout archive | digest-pinned GHCR image |

[`release-prepare.yml`](../../.github/workflows/release-prepare.yml) builds one
matrix job per descriptor build unit. Embedded frontends are built once before
the Rust target fan-out. Rust units share remote sccache objects by toolchain
and target, but artifact bytes remain target-specific.

[`release-candidate-publish.yml`](../../.github/workflows/release-candidate-publish.yml)
publishes one immutable candidate version and assigns Registry `next` with a
compare-and-swap. [`release-stable-publish.yml`](../../.github/workflows/release-stable-publish.yml)
assigns `latest` to that same version and descriptor; it does not rebuild or
create a second package version. OCI channel aliases are a separate digest-CAS
phase.

Registry publication uses the strict request:

```json
{
  "package_descriptor": {},
  "descriptor_sha256": "<package digest>",
  "tag": null,
  "repo": "https://github.com/iii-hq/workers",
  "artifacts": {}
}
```

Metadata, defaults, dependency ranges, runtime and validation come only from
the compiled package descriptor. Publish, smoke, promotion, finalize and
verify never read the root catalog or package source.
