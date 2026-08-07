# Harness E2E benchmark dashboard

This static shell replaces the generic benchmark-action index at
`dev/harness-e2e/`. The workflow-generated `data.js` remains the source of truth
for metric trends. `executions.js` indexes workflow attempts, and
`runs/<execution-id>.json` supplies the retained execution report.

Start the local dashboard from the repository root:

```bash
python3 .github/scripts/serve_harness_e2e_dashboard.py
```

The dashboard can now execute one or more scenarios against the Harness already
running at `III_URL`. It discovers registered provider/model pairs from that
stack and scenario ids from the E2E runner. The primary form only asks for an
optional label, a subject model, and scenarios; URL, judge override, run count,
and technical retries remain under **Advanced options** with safe defaults. Use
**Refresh catalog** after restarting the Harness or changing its URL. The server
runs only one experiment at a time, streams its log, imports the resulting
`results.json`, and keeps the raw local run under
`target/harness-e2e-local-runs/`.

Local executions reuse the already-built `harness/target/debug/harness-e2e`
binary, so changing and restarting the Harness does not compile the E2E client
again. On a fresh checkout, build that client once:

```bash
cargo build --locked --manifest-path harness/Cargo.toml -p harness-e2e
```

Set `HARNESS_E2E_BIN` to use a binary from another location. The dashboard never
invokes Cargo implicitly; the previous behavior is available only by explicitly
setting `HARNESS_E2E_ALLOW_BUILD=1` when starting the server.

The execution label is optional and intentionally descriptive only. The local
dashboard does not inspect or record Harness code changes: restart or modify the
Harness however you want, run another experiment, then select any two execution
rows and open **Compare selected**. The comparison always remains available;
different subjects, run counts, scenario sets, and behavioral contracts are
shown as warnings instead of blocking the comparison.

Pass files or directories to import previously saved local executions:

```bash
python3 .github/scripts/serve_harness_e2e_dashboard.py \
  harness/target/e2e-reactive-fix/results.json \
  target/harness-e2e-glm-5.2
```

Imports accumulate in `target/harness-e2e-dashboard-local`. Reimporting the same
saved report is idempotent, while every UI-triggered run has its own execution
identity. Use `--reset` to start a new local history, or `--host`, `--port`,
`--site-dir`, and `--runs-dir` to change local paths and the default
`127.0.0.1:4173` listener. Execution API writes are accepted only from a
loopback client.

To preview the sample fixtures instead, serve `.github/benchmark-site` directly:

```bash
python3 -m http.server 4173 --directory .github/benchmark-site
```

When generated data is absent, the pages load their sample fixtures and label
the view as preview data. Test both data contracts with:

```bash
node --test .github/benchmark-site/*.test.cjs
```

Metric names are stable identifiers:

```text
<quality|efficiency|reliability>::<subject>::<scenario|suite>::<metric>
```

The execution index retains 100 workflow attempts. The latest 30 also retain the
complete execution report: per-run prompts, transcripts, criteria, metrics,
costs, retries, hard gates, traces, and failure evidence. Each publish updates
the retained report metadata and removes unreferenced run files before deploying
Pages.

Each full execution summary also carries compact per-scenario averages for
tokens, wall time, cost, function calls, function-call errors, sessions, and
turns. Tokens mean input plus output; cache-read tokens are already represented
in input usage and are not added again. The execution table also exposes exact
total tokens and function calls for every retained diagnostic report.

Operational health is the primary overview. Efficiency appears after the latest
status, completeness, first actionable failure, KPIs, and scenario matrix. Its
cards show current suite totals, while deltas use only successful scenarios with the same subject,
scenario id, and behavioral contract fingerprint. New and changed scenarios
collect five comparable executions before receiving a trend verdict. Removed
scenarios remain visible as historical rows and never count as an efficiency
gain. Contract changes start a new baseline instead of joining incompatible data.
Select any scenario in the efficiency table to compare that scenario execution
by execution. The modal switches between cost, tokens, duration, function calls,
and function errors, marks contract boundaries, and links each point to its full
execution details.
