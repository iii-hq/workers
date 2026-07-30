# Harness E2E benchmark dashboard

This static shell replaces the generic benchmark-action index at
`dev/harness-e2e/`. The workflow-generated `data.js` remains the source of truth
for metric trends. `executions.js` indexes workflow attempts, and
`runs/<execution-id>.json` supplies the complete retained reports.

Open a local preview from the repository root:

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
when expanded. Each run presents its transcript as a read-only execution
conversation with message and error filters, paired function calls and results,
and recovered errors expanded by default. Diagnostic logs, stack files, and
credentials remain in access-controlled Actions artifacts.

Each full execution summary also carries compact per-scenario averages for
tokens, wall time, cost, function calls, function-call errors, sessions, and
turns. The execution home plots those values as one point per workflow attempt
across the rolling last 30 days. Selecting a scenario shows all seven metrics at
once, each in its own chart and scale. Tokens mean input plus output; cache-read
tokens are already represented in input usage and are not added again.
