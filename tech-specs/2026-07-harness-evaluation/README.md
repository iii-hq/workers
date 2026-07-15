# Harness evaluation

Architecture specifications for the two end-to-end harness evaluation tracks.

| Document | Model boundary | Primary result |
|---|---|---|
| [Conformance E2E](conformance-e2e.md) ([HTML](conformance-e2e.html)) | Scripted router or recorded replay | Deterministic pass or fail by public invariant |
| [Agent-quality E2E](agent-quality.md) ([HTML](agent-quality.html)) | Pinned real model and production provider path | Workflow quality, reliability, latency, tokens, and cost |

Conformance verifies the public harness protocol, durability, and exactly-once
effects. Agent quality measures representative workflow outcomes and makes
quality and efficiency changes visible against a pinned baseline. The tracks
share an evaluation program, but keep separate execution owners and pass
policies.

The underlying turn-loop design remains in the
[harness specification](../2026-06-agentic/harness.md).
