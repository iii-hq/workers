# Shared worker architecture

Reference material for concepts that apply across workers. For step-by-step
procedures, see [`../sops/`](../sops/).

| Document | Summary |
|---|---|
| [`worker-model.md`](worker-model.md) | Worker lifecycle, engine bus, function ids, registry discovery |
| [`iii-worker-yaml.md`](iii-worker-yaml.md) | Manifest field reference and which CI/CD jobs consume each field |
| [`deploy-modes.md`](deploy-modes.md) | `binary` / `image` / `bundle` paths through build and publish |
| [`testing-and-ci.md`](testing-and-ci.md) | PR discovery, gates, interface boot smoke, dedicated e2e workflows |
| [`skills-and-permissions.md`](skills-and-permissions.md) | `SKILL.md` lifecycle and `iii-permissions.yaml` conventions |
| [`per-worker-architecture.md`](per-worker-architecture.md) | When to add `<worker>/architecture/`; session-manager as reference |

## Per-worker architecture

Workers with non-trivial integration surfaces may ship their own
`<worker>/architecture/` folder. Example:
[`session-manager/architecture/`](../../session-manager/architecture/).
