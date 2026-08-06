# Harness quickstart validator

This check exercises the published installation path in an isolated temporary
home and project:

```bash
printf 'workers: []\n' > config.yaml
iii -c config.yaml
iii worker add harness console
```

It verifies that:

- the published installer provides a working `iii` CLI;
- a clean engine starts;
- the core harness and Console functions register;
- `console::status` and the Console HTTP root respond; and
- `config.yaml` and `iii.lock` contain the installed workers.

Run it locally with:

```bash
make -C harness quickstart-validate
```

The machine needs `curl` and `jq`. The default engine and Console ports (`49134`
and `3113`) must be available. The CLI installer and Registry worker selectors
are independent: `III_CLI_CHANNEL` chooses `latest` or `next` for `iii`, while
`III_WORKER_TAG` chooses the Registry tag used by `harness` and `console`.
The old combined `III_CHANNEL` variable is rejected to prevent a silent test
against the wrong side of the split.
Set `HARNESS_QUICKSTART_TRACE=1` to print only the important external commands
(`iii worker add`, `iii trigger`, installer, and engine) and save the list as
`commands.log`. Polling attempts, assignments, cleanup, and other shell internals
are omitted.

The nightly/manual CI workflow preserves `result.json`, the generated project
files, Console responses, raw logs, and the command trace. Release-triggered
runs replace the released worker with its exact candidate version and verify it
in `iii.lock`. The Release workflow calls quickstart and deployed E2E
synchronously; manual quickstart runs may still request the E2E cascade.

The nightly schedule runs paired `latest/latest` and `next/next` CLI/worker lanes.
Manual runs select both values explicitly. Behavioral quality remains covered
by the Harness E2E workflows.
