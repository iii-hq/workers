# Workers documentation

This folder collects **procedures** (SOPs) and **shared architecture** for
workers in this monorepo. Per-worker deep docs stay inside each worker folder
(e.g. [`session-manager/architecture/`](../session-manager/architecture/)).

## I want to…

| Goal | Read |
|---|---|
| Add a new worker to the repo | [`sops/new-worker.md`](sops/new-worker.md) |
| Scaffold a Rust `deploy: binary` worker | [`sops/binary-worker.md`](sops/binary-worker.md) |
| Ship a version to the registry | [`sops/release.md`](sops/release.md) |
| Fix a failed release | [`sops/release.md`](sops/release.md) § Troubleshooting |
| Write a consumer `README.md` | [`worker-readme.md`](../worker-readme.md) |
| Author `skills/SKILL.md` | [`DOCUMENTATION_GUIDELINES.md`](../DOCUMENTATION_GUIDELINES.md) |
| Understand `iii.worker.yaml` fields | [`architecture/iii-worker-yaml.md`](architecture/iii-worker-yaml.md) |
| Understand CI gates on a PR | [`architecture/testing-and-ci.md`](architecture/testing-and-ci.md) |
| Integrate with session-manager | [`session-manager/architecture/integration.md`](../session-manager/architecture/integration.md) |

## Layout

```text
docs/
├── README.md                 # this file
├── sops/                     # step-by-step procedures
│   ├── new-worker.md         # cross-cutting onboarding
│   ├── binary-worker.md      # Rust binary scaffold
│   └── release.md            # cut, monitor, verify releases
└── architecture/             # shared reference (not step-by-step)
    ├── worker-model.md
    ├── iii-worker-yaml.md
    ├── deploy-modes.md
    ├── testing-and-ci.md
    ├── skills-and-permissions.md
    └── per-worker-architecture.md
```

## Related docs (repo root)

- [`worker-readme.md`](../worker-readme.md) — consumer README contract
- [`DOCUMENTATION_GUIDELINES.md`](../DOCUMENTATION_GUIDELINES.md) — `skills/SKILL.md` authoring
- [`tech-specs/`](../tech-specs/) — product-level specs before or alongside implementation
