# Triage Labels

The skills speak in terms of five canonical triage roles. This file maps those roles to the actual label strings used in Linear (iii team).

| Label in mattpocock/skills | Label in Linear   | Meaning                                  |
| -------------------------- | ----------------- | ---------------------------------------- |
| `needs-triage`             | `needs-triage`    | Maintainer needs to evaluate this issue  |
| `needs-info`               | `needs-info`      | Waiting on reporter for more information |
| `ready-for-agent`          | `ready-for-agent` | Fully specified, ready for an AFK agent  |
| `ready-for-human`          | `ready-for-human` | Requires human implementation            |
| `wontfix`                  | `wontfix`         | Will not be actioned                     |

`needs-triage` already exists on the iii team. The other four don't yet — create each with `create_issue_label` (team `iii`) the first time it's needed; don't repurpose `Needs Discussion` or `blocked`, which mean different things.

When a skill mentions a role (e.g. "apply the AFK-ready triage label"), use the corresponding label string from this table.
