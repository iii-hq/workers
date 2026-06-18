# Permission sources (inventory)

Before the single-source consolidation, four places could disagree about
what an agent may do:

| Source | Location | What it holds |
|---|---|---|
| **Deployment rules** | `configuration` entry `approval-gate` → `rules` (+ legacy `always_allow_seed`, now folded into mode-scoped allow rules) | First-match `allow` / `deny` / `hold` for every function call |
| **Per-session deltas** | `state` scope `approval_settings/<session_id>` → `mode`, `always_allow`, `approved_always` | Human choices layered on top of deployment defaults |
| **Console defaults (removed)** | ~~`localStorage` `iii-default-permission-mode` / `iii-default-allowlist`~~ | Was a fifth copy; console now reads/writes the `approval-gate` entry |
| **Harness structural floor** | Per-turn `FunctionPolicy { allow, deny, expose }` on `harness::send` | Fail-closed globs before the pre_trigger hook chain; derived from deployment rules at send time |

## Canonical source

**`approval-gate` configuration `rules`** is the single deployment policy.
Auto-mode trust that used to live in `always_allow_seed` is expressed as
`allow` rules with `"modes": ["auto"]`. Session `always_allow` /
`approved_always` remain per-session human deltas.

## Consumers

| Player | How it implements the one source |
|---|---|
| **approval-gate** | Evaluates `rules` (+ session deltas) in `approval::gate` |
| **Harness** | `FunctionPolicy` on send derived from the same rules; `approval::gate` bound as `pre_trigger` |
| **Console** | Edits `default_mode` + auto allowlist via `configuration::get/set` on `approval-gate` |
