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
and `3113`) must be available. The default installer channel is `latest`; set
`III_CHANNEL=next` to validate `next`.
Set `HARNESS_QUICKSTART_TRACE=1` to print only the important external commands
(`iii worker add`, `iii trigger`, installer, and engine) and save the list as
`commands.log`. Polling attempts, assignments, cleanup, and other shell internals
are omitted.

The nightly/manual CI workflow preserves `result.json`, the generated project
files, Console responses, raw logs, and the command trace. Release-triggered
runs also verify the exact released worker version in `iii.lock` before
dispatching the deployed Harness E2E workflow.

The nightly schedule runs both `latest` and `next` as independent matrix jobs.
Manual runs select one of those channels through the workflow input. Behavioral
quality remains covered by the Harness E2E workflows.
