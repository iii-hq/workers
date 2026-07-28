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

When `ZAI_API_KEY` is set, it also installs `provider-zai`, resolves
`zai/glm-5.2`, sends a real Harness message through the Console `/ws` proxy,
waits for the turn to complete, and requires a non-empty assistant reply.

Run it locally with:

```bash
make -C harness quickstart-validate
```

The machine needs `curl` and `jq`; the GLM canary additionally needs
`python3` with `venv` support. The default engine and Console ports (`49134`
and `3113`) must be available. Set `III_CHANNEL=next` to validate the `next`
installer channel. `HARNESS_QUICKSTART_MODEL` overrides the default GLM model.

The nightly/manual CI workflow preserves `result.json`, the generated project
files, Console responses, and raw logs. In CI a
[VHS](https://github.com/charmbracelet/vhs) tape records the real validator
operations and renders `quickstart.mp4` (the same recording pattern as
`iii-hq/templates`). The tape waits for a unique shell prompt so completion
does not depend on matching scrolling output. VHS cannot propagate the typed
command's exit code, so the workflow reads pass/fail from `result.json`;
recording failures do not repeat a completed live GLM request or override its
result. Each run also creates one
`#worker-releases` Slack message, updates it with the final status, posts the
result details in its thread, and uploads the terminal recording to the same
thread (pass or fail; requires the `files:write` bot scope). This uses the
organization-level `SLACK_BOT_TOKEN`; the bot must be invited to the channel.
Notification errors are reported as workflow warnings without blocking
validation.

Without `ZAI_API_KEY`, the live canary is recorded as `skipped`. Behavioral
quality remains covered by the Harness E2E workflows.
