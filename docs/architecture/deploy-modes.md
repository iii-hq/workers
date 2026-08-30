# Release artifact modes

The Workers-owned compiler translates one private build entry plus its public
manifest into an immutable release descriptor and independent build units.
The release train consumes only that descriptor after prepare.

| `artifact.kind` | Prepared artifact | Publication |
|---|---|---|
| `rust-binary` | one deterministic `tar.gz` and checksum per target | GitHub Release URLs and digests keyed by Rust target |
| `javascript-bundle` | one deterministic archive from explicit files | GitHub Release archive URL and digest |
| `python-bundle` | one deterministic archive from explicit files | GitHub Release archive URL and digest |
| `oci-image` | deterministic OCI-layout archive | digest-pinned GHCR image |

[`deploy-prepare.yml`](../../.github/workflows/deploy-prepare.yml) builds one
matrix job per descriptor build unit. Embedded frontends are built once before
the Rust target fan-out. Rust units share remote sccache objects by toolchain
and target, but artifact bytes remain target-specific.

[`deploy-candidate-publish.yml`](../../.github/workflows/deploy-candidate-publish.yml)
publishes one immutable candidate version and assigns Registry `next` with an
idempotent, verified update. [`deploy-stable-publish.yml`](../../.github/workflows/deploy-stable-publish.yml)
assigns `latest` to that same version and descriptor; it does not rebuild or
create a second package version. OCI channel aliases are a separate digest-CAS
phase.

Registry publication projects the descriptor onto the current API. For a
binary worker the request has this shape:

```json
{
  "worker_name": "state",
  "version": "0.22.3-rc.3",
  "type": "binary",
  "tag": "next",
  "description": "...",
  "license": "Apache-2.0",
  "tags": [],
  "dependencies": [],
  "config": {},
  "functions": [],
  "triggers": [],
  "repo": "https://github.com/iii-hq/workers",
  "binaries": {}
}
```

Candidate publication assigns `next` atomically in `POST /publish` and proves
both the exact version and channel through current Registry read surfaces.
Descriptor-only fields such as `package_descriptor` and `descriptor_sha256`
are never sent to the current Registry. Publish, smoke, promotion, finalize,
and verify never read the private catalog, public manifest, or package source.
