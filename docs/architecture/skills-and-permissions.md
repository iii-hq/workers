# Skills and permissions

How agent-facing skill docs and default permissions are managed.

## Skill documentation lifecycle

### Authoring

Each worker may ship `skills/SKILL.md` — a lean intent doc for agents (when to
use, boundaries, function catalogue). It is optional, including when other
markdown documents exist under `skills/`. **Not** JSON schemas or worked
examples; those live in `iii get function info`.

Author per [`DOCUMENTATION_GUIDELINES.md`](../../DOCUMENTATION_GUIDELINES.md).

### PR validation

Skill documents are optional for every worker. PR validation does not require a
canonical `skills/SKILL.md` entrypoint.

### Publish

On every successful release (when `interface_smoke != false`):

1. `build_skills_payload.py` collects any non-empty markdown under `skills/`
2. `POST /w/<worker>/skills` — skipped cleanly when no markdown found

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

For the configuration worker, `configuration::list` and
`configuration::schema` are read-only and allowed. `configuration::get` is a
sensitive read and needs approval by default. Agents cannot call
`configuration::set` or `configuration::register`. Direct Console
configuration pages and worker-to-worker configuration calls use privileged
paths; this agent policy does not change them.

Update [`iii-permissions.yaml`](../../iii-permissions.yaml) when adding a worker
whose functions agents should or should not call without approval.

## Related

- New worker checklist: [`../sops/new-worker.md`](../sops/new-worker.md) §7–§8
- session-manager integration permissions: [`session-manager/architecture/integration.md`](../../session-manager/architecture/integration.md) §2
