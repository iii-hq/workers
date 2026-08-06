# Harness E2E benchmark dashboard

This static shell replaces the generic benchmark-action index at
`dev/harness-e2e/`. The workflow-generated `data.js` remains the source of truth
for metric trends. `executions.js` indexes workflow attempts, and
`runs/<execution-id>.json` supplies the complete retained reports.

Import the default local report and serve the real dashboard from the repository
root:

```bash
python3 .github/scripts/serve_harness_e2e_dashboard.py
```

Pass files or directories to import other local executions:

```bash
python3 .github/scripts/serve_harness_e2e_dashboard.py \
  harness/target/e2e-reactive-fix/results.json \
  target/harness-e2e-glm-5.2
```

Imports accumulate in `target/harness-e2e-dashboard-local`. Reimporting the same
report is idempotent. Use `--reset` to start a new local history, or `--host`
and `--port` to change the default `127.0.0.1:4173` listener. The command only
reads existing reports; it does not run E2E scenarios.

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
complete structured `results.json` content, including prompts, transcripts,
session ids, gates, criteria, failures, retries, usage, and traces. The UI loads
those reports only on the detail page and renders transcript-heavy sections only
when expanded. Each run opens its transcript in a read-only dialog patterned
after the Harness chat, with message and error filters, paired function calls
and results, and recovered errors expanded by default. Diagnostic logs, stack
files, and credentials remain in access-controlled Actions artifacts.

Each full execution summary also carries compact per-scenario averages for
tokens, wall time, cost, function calls, function-call errors, sessions, and
turns. Tokens mean input plus output; cache-read tokens are already represented
in input usage and are not added again. The execution table also exposes exact
total tokens and function calls for every retained full report.

Efficiency is the primary overview. Its cards show the current operational suite
totals, while deltas use only successful scenarios with the same subject,
scenario id, and behavioral contract fingerprint. New and changed scenarios
collect five comparable executions before receiving a trend verdict. Removed
scenarios remain visible as historical rows and never count as an efficiency
gain. Contract changes start a new baseline instead of joining incompatible data.
Select any scenario in the efficiency table to compare that scenario execution
by execution. The modal switches between cost, tokens, duration, function calls,
and function errors, marks contract boundaries, and links each point to its full
execution details.
