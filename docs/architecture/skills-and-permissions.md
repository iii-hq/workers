# Skills and permissions

How agent-facing skill docs and default permissions are managed.

## skills/SKILL.md lifecycle

### Authoring

Each worker may ship `skills/SKILL.md` — a lean intent doc for agents (when to
use, boundaries, function catalogue). **Not** JSON schemas or worked examples;
those live in `iii get function info`.

Author per [`DOCUMENTATION_GUIDELINES.md`](../../DOCUMENTATION_GUIDELINES.md).

### PR validation

| Case | Rule |
|---|---|
| Bootstrap workers (`shell`, `iii-directory`) | `skills/SKILL.md` **required**, non-empty, ≤ 256 KiB |
| Other workers | Optional; validated only if present |

Bootstrap list: `BOOTSTRAP_WORKERS` in
[`validate_worker.py`](../../.github/scripts/validate_worker.py) — the workers
whose skills the harness stack requires at boot. Keep it in sync with what the
harness actually bootstraps when that set changes.

### Publish

On every successful release (when `interface_smoke != false`):

1. `build_skills_payload.py` collects `skills/SKILL.md` and `skills/<rel>.md`
2. `POST /w/<worker>/skills` — skipped cleanly when no markdown found

### Out-of-band republish

[`publish-worker-skills.yml`](../../.github/workflows/publish-worker-skills.yml)
updates skills without a version bump. Worker must be in `ALLOWED_WORKERS`
([`parse_publish_workers_input.py`](../../.github/scripts/parse_publish_workers_input.py)).

**Drift note:** `email` ships skills but is not yet in `ALLOWED_WORKERS`.

## iii-permissions.yaml

Repo-root [`iii-permissions.yaml`](../../iii-permissions.yaml) defines default
agent-callable surfaces. Harness loads it via `permissions_path` and hot-reloads
on save.

### Rule syntax

```yaml
rules:
  - '!function_id'    # deny (quote required — bare ! is YAML tag syntax)
  - 'worker::*'       # allow glob
  - function_id       # allow exact match
```

**First match wins.** No match → `needs_approval`.

### Conventions for new workers

| Surface type | Default |
|---|---|
| Transcript / config mutators | Deny (`!session::append`, `!configuration::set`, …) |
| Operator / health signals | Deny (`!shell::config-status`, internal reload hooks) |
| Read-only introspection | Allow (`engine::functions::list`, `coder::read-file`, …) |
| Sensitive reads | Leave unmatched → approval-gated |

Precedent: `session-manager` block — deny all writes and `session::store::*`;
reads stay at default approval.

Update [`iii-permissions.yaml`](../../iii-permissions.yaml) when adding a worker
whose functions agents should or should not call without approval.

## Related

- New worker checklist: [`../sops/new-worker.md`](../sops/new-worker.md) §7–§8
- session-manager integration permissions: [`session-manager/architecture/integration.md`](../../session-manager/architecture/integration.md) §2
