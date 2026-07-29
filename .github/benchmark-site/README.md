# Harness E2E benchmark dashboard

This static shell replaces the generic benchmark-action index at
`dev/harness-e2e/`. The workflow-generated `data.js` remains the source of
truth; `dashboard-data.js` normalizes its two benchmark groups into one release
view.

Open a local preview from the repository root:

```bash
python3 -m http.server 4173 --directory .github/benchmark-site
```

When `data.js` is absent, the page loads `sample-data.js` and labels the view as
preview data. Test the data contract with:

```bash
node --test .github/benchmark-site/dashboard-data.test.cjs
```

Metric names are stable identifiers:

```text
<quality|efficiency|reliability>::<subject>::<scenario|suite>::<metric>
```

Keep the published dataset aggregate-only. Raw prompts, transcripts, logs, and
judge attempts belong in private Actions artifacts, never in this directory or
the `gh-pages` history.
