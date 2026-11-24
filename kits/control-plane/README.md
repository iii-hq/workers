# control-plane

This kit installs the hidden `control-plane-writer` profile used by the `control-plane`
worker. The profile produces referenced plain-text summary, diary,
run and pull-request notes. Live facts, counts and durations remain the
worker's responsibility.

The kit intentionally installs no worker, policy, schema or trigger. Apply it
through Directory, configure `control-plane.summary.model`, and let the worker create
and schedule `control-plane-<scope>` automation sessions. The worker supplies the
read-only function policy and structured result schema on every send.

GitHub is optional. The worker omits unavailable pull-request facts while the
writer still covers kanban, harness and session sources.
