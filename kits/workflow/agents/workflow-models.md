---
name: Workflow Models
description: "Data for the workflow kit, not an agent: the models each stage of a run uses, per provider, best first. Edit the table to change them."
hidden: true
---
# Workflow models

This file is data for the workflow kit, not an agent: the Workflow agent reads the table below when a run starts. A session started on this profile only answers that `workflow-models` is not an agent.

One row per provider, named as the `provider` field of `router::models::list` names it. A run uses the row of the provider the Workflow agent's own model belongs to. Each cell lists model ids, best first, separated by commas: the first one the catalog has, with tools, is that stage's model, and an id the catalog does not have is skipped. With a row, nothing outside it is picked: a stage whose cell matches nothing runs on the Workflow agent's own model. A provider with no row keeps the kit's automatic choice.

- `plan`: the design, refute and plan nodes, the review's refuters, the work or code node of a ticket the plan marks hard, and the repair of a batch whose check stalled.
- `check`: every ticket and batch check, and the critic.
- `work`: the explore nodes, the baseline and the plain flow's work nodes: reading, running commands and reporting.
- `code`: every node that changes files: the code and solo nodes and the batch's tests and repairs. A row with no `code` cell uses its `check` cell.

| provider | plan | check | work | code |
|---|---|---|---|---|
| claude-code | claude-code/claude-fable-5-2, claude-code/claude-opus-5-5 | claude-code/claude-opus-5-5 | claude-code/claude-sonnet-5-5, claude-code/claude-opus-5-5 | claude-code/claude-opus-5-5 |
| openai-codex | codex/gpt-6.2-astra, codex/gpt-6-astra, codex/gpt-6.2-sol, codex/gpt-6.1-sol | codex/gpt-6.2-sol, codex/gpt-6.1-sol, codex/gpt-6-sol | codex/gpt-6.2-terra, codex/gpt-6.1-terra | codex/gpt-6.2-sol, codex/gpt-6.1-sol, codex/gpt-6-sol |

Edit, add or remove rows freely; kit updates keep your edits. An id that fails when a run uses it is skipped for the rest of that run: its stages move to the next id of their cell, else to the Workflow agent's own model. Every new run tries it again, so remove an id your account cannot run, or pass `skip_models=<id>`. A profile pin and the `work_model`, `check_model` and `plan_model` args win over this table, and `models=own` turns it off.
