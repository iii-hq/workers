# Harness quickstart validator

This validator exercises the published installation path without provider
credentials or model calls. It runs in an isolated temporary home and project
directory, installs the `iii` CLI, starts a clean engine, and follows the
documented commands:

```bash
printf 'workers: []\n' > config.yaml
iii -c config.yaml
iii worker add harness console
```

The check waits for the engine and the core harness/Console function surface,
calls `console::status`, and fetches the Console HTTP root. It also verifies
that `config.yaml` and `iii.lock` were produced by the registry install.

Run it locally with:

```bash
make -C harness quickstart-validate
```

The local machine needs `curl` and `jq`; CI provides both on the standard
Ubuntu runner.

The CI workflow runs this check nightly and manually against the `latest`
registry artifacts. It intentionally does not test model generation; provider
credentials and model quality are covered by the Harness E2E workflow.
