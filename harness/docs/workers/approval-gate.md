# approval-gate (moved)

The in-harness approval-gate worker has been removed. Approval policy —
permission modes, allow-lists, the yaml-policy fallback, the pending inbox,
and the decision RPCs (`approval::resolve`, `approval::set-mode`, …) — now
lives in the **standalone approval-gate worker** (Rust crate at
[`approval-gate/`](../../../approval-gate/) in the repo root).

- Spec: [tech-specs/2026-06-agentic/approval-gate.md](../../../tech-specs/2026-06-agentic/approval-gate.md)
- Crate docs: [approval-gate/README.md](../../../approval-gate/README.md)

The harness keeps only the mechanics the gate composes with:

- the `harness::hook::pre-trigger` trigger type the gate binds
  `approval::gate` to (see
  [turn-orchestrator.md § Hook chokepoint](turn-orchestrator.md#hook-chokepoint)),
- `harness::function::resolve` (`execute` releases a held call, `deliver`
  answers it without executing), and
- the `harness::turn-completed` trigger type (terminal-turn cleanup).

Deployment note: the gate is a separate process (like `llm-router`). Start
the harness first — the gate's `harness::hook::pre-trigger` binding is
best-effort at boot and only retries on gate restart. A deployment without
the gate runs **ungated** (zero pre_trigger bindings ⇒ every call is
allowed).
