# Harness quickstart validator

This validator exercises the published installation path. It runs in an
isolated temporary home and project directory, installs the `iii` CLI, starts
a clean engine, and follows the documented commands:

```bash
printf 'workers: []\n' > config.yaml
iii -c config.yaml
iii worker add harness console
```

The check waits for the engine and the core harness/Console function surface,
calls `console::status`, and fetches the Console HTTP root. It also verifies
that `config.yaml` and `iii.lock` were produced by the registry install.

When `ZAI_API_KEY` is set, the validator goes one step further: it adds the
Z.AI provider (`iii worker add provider-zai`), then sends a real message
through the Console's `/ws` proxy — the same WebSocket path the browser SPA
uses — via `console_send.py`, and asserts the turn completes with a non-empty
assistant reply (default model `glm-5.2`, overridable with
`HARNESS_QUICKSTART_MODEL`/`HARNESS_QUICKSTART_PROVIDER`). Without the key the
live check is skipped and recorded as such in `result.json` and `EVIDENCE.md`.

Set `III_CHANNEL=next` to validate the `next` installer channel instead of
`main`; in CI this is exposed as the `channel` input on manual dispatches.

Run it locally with:

```bash
make -C harness quickstart-validate
```

The local machine needs `curl` and `jq`; CI provides both on the standard
Ubuntu runner.

The run narrates every step with timestamped log lines and `[ok]` assertions,
records per-stage durations in `timings.tsv` (rendered as a `[timing breakdown]`
at the end and embedded in `result.json`), and always writes an `EVIDENCE.md`
digest — status, CLI version, timing, and the tail of every log — into the
artifacts directory, on success and on failure alike. In CI the evidence file
becomes the job step summary and the raw logs are printed in collapsible
groups, so a run can be audited without downloading anything.

The CI workflow runs this check nightly and manually against the `latest`
registry artifacts. It intentionally does not test model generation; provider
credentials and model quality are covered by the Harness E2E workflow.
