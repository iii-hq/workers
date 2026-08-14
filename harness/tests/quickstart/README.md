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
- `ANTHROPIC_API_KEY` exposes exactly `anthropic/claude-sonnet-5` and
  `OPENAI_API_KEY` exposes exactly `openai/gpt-5.6-luna`;
- `console::status` and the Console HTTP root respond;
- a user can complete a message with Claude Sonnet 5, switch to GPT-5.6 Luna
  in the same chat, and complete another message without changing sessions;
- a user can create a new chat and complete a third message with GPT-5.6 Luna;
- both successful conversations survive a browser reload and reach durable
  terminal Harness states;
- in a separate chat, a user can invoke one real `shell::exec` capability,
  observe its successful Console card and exact output, and recover the same
  conversation after a reload;
- the capability call and its exact output are present in the durable session
  transcript; and
- `config.yaml` and `iii.lock` contain the installed workers.

Run it locally with:

```bash
export ANTHROPIC_API_KEY='<your-anthropic-api-key>'
export OPENAI_API_KEY='<your-openai-api-key>'
make -C harness quickstart-validate
```

The machine needs `curl`, `jq`, `ffmpeg`, the `console/web` dependencies, and
Chromium for Playwright. The validator invokes the repository's local
Playwright binary directly, so Corepack cannot silently select a different
pnpm version while the quickstart is running.
The default engine and Console ports (`49134` and `3113`) must be available.
The CLI installer and Registry worker selectors are independent:
`III_CLI_CHANNEL` chooses `latest` or `next` for `iii`, while `III_WORKER_TAG`
chooses the Registry tag used by `harness` and `console`. The old combined
`III_CHANNEL` variable is rejected to prevent a silent test against the wrong
side of the split.
Set `HARNESS_QUICKSTART_TRACE=1` to print only the important external commands
(`iii worker add`, `iii trigger`, installer, and engine) and save the list as
`commands.log`. Polling attempts, assignments, cleanup, and other shell internals
are omitted.

The CI workflow preserves `result.json`, the generated project files, sanitized
browser, terminal, and first-capability evidence, raw Playwright output, the
command trace, and the provider-switch MP4. It scans every artifact for the
literal Anthropic and OpenAI credentials before upload. Release-triggered runs
replace the released worker with its exact candidate version and verify it in
`iii.lock`.

After a successful `deploy_harness`, Release Control dispatches this check as a
child observation. Its result and MP4 are appended to the release Slack thread,
but cannot change the already successful deployment result. A manually created
quickstart operation gets its own Slack root. Slack delivery retries through the
notification outbox and does not fail the test operation.
