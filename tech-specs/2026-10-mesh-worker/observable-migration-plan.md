# Mesh observer API migration implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox syntax for tracking.

**Goal:** Make Mesh work with the current main engine and observer protocol, proving it on a separate engine without disturbing the running instance.

**Architecture:** Use the same Go SDK and observable-function patterns as the other current Go workers. Replace the removed trigger protocol across Mesh production code and integration fixtures. Keep node permissions, ownership checks, revocation, namespace boundaries and native service behavior intact.

**Tech Stack:** Go 1.27.1+, iii Go SDK v0.26.0-rc.5 (as in whatsapp/go.mod), current in-tree rbac-proxy and worker binaries, Node/pnpm for embedded UI, Rust 1.97.1.

**Spec:** User instruction: “a ideia é subir a engine usando outra porta para não atrapalhar a que já está rodando. Fazer a correção para funcionar com a nova api de observables”. Original functional/security requirements: tech-specs/2026-10-mesh-worker/mesh.md and README.md. Current protocol: docs/architecture/worker-model.md; docs/sops/configuration.md section 5; docs/sops/injectable-console-ui.md; current SDK source; whatsapp/observe.go and ui.go; rbac-proxy/src/interceptor.rs on_observe.

## Global Constraints

- Existing linked worktree spec/mesh-worker; starting commit d82241fd113fefcd1ce0357817b46a8826b98801. No push, PR or ticket.
- Never stop, reconfigure, seed, register workers on or otherwise mutate the existing engine on 127.0.0.1:49134 (observed PID 201913). Test engines bind loopback on fresh ephemeral ports; their config/state/logs and proxy listeners are separate and are cleaned up by their owner.
- Preserve the original network defaults and native behavior in product code. Validation is local only: no native service install, sudo, TUN/route/hosts edits, native mounts, external tunnels or publication.
- Preserve signed node requests, configuration ownership <node>__<worker>, raw-only reads, same-node observer confinement, shared-state/harness/queue/http/cron/ADE injection refusals, operator extra grants and revocation.
- Explicitly disable new SDK telemetry export on Mesh-owned clients; a test-only environment override must not conceal production defaults.
- No compatibility shim or fallback to the removed trigger wire protocol. Historical original specs/evidence stay historical; update active README, permissions/skills and current tests as needed.
- Real protocol regression tests must cover observable deliveries and refusals. Do not weaken assertions or skip required tests to obtain green.
- Node-event no-history applies to queued/unadmitted events. Logical callback admission occurs at the post-match live-binding/generation check under its delivery lock. An already-admitted SDK local invocation can enqueue its callback after reconnection to the same retained observer; the SDK exposes no atomic epoch-pinned cancellation. Keep this limit explicit; changed/removed generations must still fail validation before admission and drain before replacement acknowledgements.
- Follow TDD, meaningful focused checks during iteration; run Go fmt/vet/tests/build and affected Rust/UI checks once at completion of relevant source changes. Use CGO_ENABLED=0 GOWORK=off GOFLAGS=-mod=readonly GOTOOLCHAIN=local TS_NO_LOGS_NO_SUPPORT=true for normal Go checks.

## Review Focus

- Source identity collision: a bare source sharing an unprefixed hub function name must not observe foreign data; canonical same-node identity and namespace must be enforced.
- Configuration observer filter: missing, broad, malformed, cross-node, foreign-namespace, condition-like or injected args must not widen visibility beyond owned configuration; positive same-node delivery must work.
- Private events and connection lifecycle: no observer means no replay/history; accepted/removed/reconnected observers and node filters must behave correctly without stale callbacks or unbounded growth.
- Actual UI and hot reload: registrations reach ade::ui::asset, configuration updates trigger re-fetch, defaults/project namespaces work, and malicious node assets remain refused.
- Test isolation and actual runtime: use rebuilt current binaries, assert successful proxy ownership and prefixed worker calls; no connections to the operator engine or hidden skips; reports must not publish credentials.

### Task 1: Migrate the runtime and protocol boundaries

**Files:** Modify mesh/go.mod/go.sum; Mesh Go production and adjacent unit/protocol tests under mesh/internal/hub, internal/node and main.go as needed; mesh/iii-permissions.yaml and root iii-permissions.yaml plus their affected tests; rbac-proxy/src/interceptor.rs and tests only if canonical hook identity is needed. Keep existing module responsibilities. Rename triggerhook.go/tests to observerhook.go/tests if that responsibility changes. No E2E-tag suite migration in this task (Task 2 owns it).

**Interfaces:** Consumes current SDK v0.26.0-rc.5 and rbac on_observer_registration_function_id. Produces ordinary SDK Call usage, configuration::on-updated observers with DeliverOutput and owned input.id/event_type filters, ade::ui::asset observers, and private observable mesh::on-node-changed (optional observer args node, unchanged event payload name/event/at). Proxy hook endpoint is mesh::on-observer-registration. Consumers use Observe, not RegisterTrigger. No removed auth trigger fields.

- [x] Reproduce TestRunningProxyMeshClaim failure with current built proxy and iii 0.26.0-rc.5 in its isolated ports. Existing confirmed RED: proxy applied configuration does not match mesh ownership; proxy logs ignored on_trigger_registration_function_id.
- [x] Read the current SDK and production references. Write/port behavioral tests that fail before implementation for proxy ownership, owned config observation delivery and refusals, canonical source collisions and namespaces, node event filtering/lifecycle and ADE registrations. Use real SDK protocol frames/engine where required; fixtures must speak the actual protocol, not old frame names.
- [x] Update SDK and migrate normal RPC call types, registration return handles, observer startup/lifetime, config/UI subscriptions, event source registration/emission, proxy ownership/hook payload policy and auth surface. Preserve security constraints; if the hook requires canonical identities, extend the proxy payload compatibly and test that it reflects actual registrations rather than trusting bare IDs. Keep raw id compatibility fields only where current contract needs them, never revive trigger frames.
- [x] Auto-upgrade only a known legacy Mesh-owned hook setting (auth mesh::auth, middleware mesh::middleware, legacy hook mesh::on-trigger-registration, no conflicting hooks/exposed grants). Replace it with on_observer_registration_function_id=mesh::on-observer-registration, preserving every unrelated operator field and endpoint. Test existing-owned upgrade and foreign/conflicting refusal; do not take over ownership or silently repair endpoints.
- [x] Maintain node event schema, optional node filter, best-effort/no-history semantics and existing resource bounds where applicable under the observer model. Unknown args rejected; actual lifecycle event transitions preserved.
- [x] Update active internal deny entries to the new hook/event IDs and run permission checks. Update normal tests to compile with the new SDK and pin behavior.
- [x] Run gofmt, Go vet/tests/build; targeted real TestRunningProxyMeshClaim with built current rbac-proxy, and applicable proxy Rust checks/tests if changed. E2E-tag compilation may remain for Task 2; record it explicitly. Commit and self-review.

### Task 2: Prove the full migration with isolated end-to-end tests and document use

**Files:** Modify mesh/tests/e2e Go fixtures/cases/observation reports, runner and public report projection/tests as needed; .github/workflows/mesh-e2e.yml if required; mesh/README.md and mesh/skills/SKILL.md plus active skill references if applicable. Product fixes uncovered by real E2E belong here with a failing behavioral reproduction. Update tracked delivery evidence with an additive dated section; keep old evidence historical.

**Interfaces:** Consumes migrated runtime from Task 1: Call, Observe, mesh::on-node-changed, mesh::on-observer-registration, configuration::on-updated and ade::ui::asset. Produces current-protocol real integration/adversarial coverage and a reproducible local test command on a separate engine.

- [x] Port the E2E-tag fixtures/assertions to the current SDK and observer frames. Verify real registrations, deliveries and refusals, not just compile success. Retain the intent of all existing node/worker/revocation/security cases; replace obsolete trigger-only conditions with the corresponding observer args/filters/canonical-source attacks.
- [x] Add/verify actual positive owned configuration notification and own event observation plus cross-node/namespace/shared producer/ADE injection negative cases; demonstrate hot-reload and node-changed delivery on the new engine. Ensure connection-correlated observations use observer IDs and redact credential-bearing data in the public projection.
- [x] Build current auxiliary workers/UI assets needed by the runner. Start engines only in isolated loopback ports/temp state. Run the full Mesh E2E runner with iii 0.26.0-rc.5 (or exact same compatible version as SDK); no required skips. Respect controlled registry fixtures and current desktop limitations.
- [x] Update README/skill examples and hooks to the current observable API, explain how to build/run/test with an explicit alternative engine port and isolated working directory; retain defaults but never target the running engine in test examples. State publication/native acceptance limits accurately.
- [x] Run relevant Go suite/build/vet, E2E, UI build and public-report tests; Rust checks only for affected source/new runtime failures. Verify the protected engine still has its original PID/listener and git diff --check. Record exact command/output/evidence paths and new limitations. Commit and self-review.
