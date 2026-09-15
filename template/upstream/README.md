# harness + console: a base compose template

The smallest compose project that gives you a working iii agent harness and the
web console.

## Setup your harness authentication (API Key or Provider Login)

Export the key in the shell you start the compose daemon from:

If you need to provide an API key you can either set one in `.env` or
your environment. Either works.

```bash
export ANTHROPIC_API_KEY=sk-ant-...
export OPENAI_API_KEY=sk-...
```
## Start the compose worker and bring the iii engine up in one command

In a new terminal from the project directory run:

```bash
iii compose --up
```

## Start the harness via worker-compose.yaml

Compose can also be controlled like any other iii worker, for example here's what you
would run if you started `iii compose` without the `--up flag`:

```bash
iii trigger compose::up --namespace default file=./worker-compose.yaml --timeout-ms 300000
```

## Use the harness

Once the harnesses is started you should see output from the `iii compose` daemon like:

```bash
$ iii compose --namespace default
compose serving
  engine: ws://127.0.0.1:49134
  namespace: default
  start a project: iii trigger compose::up --namespace default file=./worker-compose.yaml
[compose] project /Users/tony/iii/projects/testing/compose/harness/worker-compose.yaml loaded into default
✓ state ready (1.4s)
✓ queue ready (1.2s)
✓ cron ready (1.2s)
✓ ide ready (962ms)
✓ session-manager ready (1.2s)
✓ iii-directory ready (1.2s)
✓ llm-router ready (1.7s)
✓ provider-anthropic ready (2.1s)
✓ provider-openai ready (2.1s)
✓ provider-openai-codex ready (2.1s)
✓ context-manager ready (2.0s)
✓ harness ready (6.6s)
✓ ade ready (956ms)
up: 13 of 13 changed in 22.8s
```

Once you see that output open the console at **http://127.0.0.1:3113**. It's all setup and ready for you
to start developing iii applications with agentic assistance.

## About this project

The `worker-compose.yaml` file in this project specifies how to start the entire
system that supports the harness.

The first `compose::up` downloads workers into `~/.iii/compose/packages`. After
that they are cached.

### What is in it, and why

| Tier | Containers                                                            | Why                                                                                 |
| ---- | --------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| 1    | `state`, `queue`, `cron`, `shell`, `session-manager`, `iii-directory` | Direct `harness` dependencies with no dependencies of their own                     |
| 2    | `llm-router`                                                          | Model routing. Needs `state`                                                        |
| 3    | `provider-anthropic`, `provider-openai`, `provider-openai-codex`, `context-manager` | `harness` names both providers explicitly, so both are required even if you use one |
| 4    | `harness`                                                             | The turn loop                                                                       |
| 5    | `console`                                                             | The web UI                                                                          |
| 6    | `browser`                                                             | Chromium sessions and one-shot HTTP fetches (`browser::fetch`) the profiles verify with |

## Agent profiles: a hierarchy that coordinates through spawn and state

`agents/` ships five profiles the console's agent picker lists (or
`harness::send { options: { agent: "<id>" } }` runs). Each extends the
harness's built-in `iii-minimal` identity and preloads its skills from
`skills/harness/…`; the harness freezes both into every session that runs as
that profile.

```text
iii-minimal
├── ade-worker-builder        plans the spec with you, briefs a Tech Lead, accepts in the console
│   └── tech-lead             writes the architecture, briefs the engineers, verifies the seam
│       ├── backend-engineer  the Node worker: functions, trigger types, configuration, UI delivery
│       └── frontend-engineer the UI the worker injects into the console
└── agent-profile-creator     plans a new profile with you and writes it beside these
```

| Profile id | Role |
| --- | --- |
| `ade-worker-builder` | Interviews you until an ADE worker's spec is unambiguous (`specs/<worker>.md`), hands it to a Tech Lead, and accepts only after exercising every criterion in the running console. |
| `tech-lead` | Turns the spec into an architecture (granular functions, reactive trigger types, one home per fact, the console surface), runs the Backend Engineer then the Frontend Engineer, and verifies the seam in a browser session. |
| `backend-engineer` | Owns the package boilerplate (`package.json`, `scripts/dev.mjs`, `ui/build.mjs`, asset delivery, the `compose::add` declaration) that gives both halves hot reload under `pnpm dev`, builds the Node worker per the `iii-node` skill, and verifies every function with a real call. |
| `frontend-engineer` | Builds the injected console UI against `@iii-dev/console-ui` and verifies it in the running console at every width and theme. |
| `agent-profile-creator` | Plans a new profile with you, using the existing ones as the reference, and writes `agents/<id>.md`. |

There is no board and no message bus between agents. Orchestration is the
`harness/orchestration` skill, two wires with one direction each:

- **Downstream is `harness::spawn`.** The `task` is the child's whole brief:
  the spec or architecture file by path, the project root, what is out of
  scope, the checks that mean done, and the state key for its result. A
  child that must spawn children of its own (the Tech Lead) is spawned with
  `options: { orchestrator: true }`.
- **Upstream is `state`.** The child writes one result document
  (`outcome`, `summary`, `evidence`, `files`, `questions`) to scope
  `results`, key `<its session id>`, and stops. The parent armed a `state`
  wake on that key before spawning, so the write starts its next turn. The
  child never looks for a parent; feedback comes back as a new task in the
  same session (`harness::spawn` with the same `session_id`).
- Results are visible on the console's state page (`#/ext/state-manager`),
  scope `results`.

The profiles ship without a `model`, so each session takes the model of the
send. To pin one, add `model: <provider>::<model>` (and optionally
`reasoning_effort`) to a profile's frontmatter; `router::models::list` prints
the catalog. Skill ids are prefixed `harness/` because `iii-directory` only
lists skills whose namespace is a worker in this compose file.

## Credentials

`worker-compose.yaml` has a ready-to-uncomment container block for every
provider below, plus a matching environment variable. Adding one
is: uncomment the worker, set its environment variable, start compose.

| Provider              | Environment variable |
| --------------------- | -------------------- |
| `provider-anthropic`  | `ANTHROPIC_API_KEY`  |
| `provider-openai`     | `OPENAI_API_KEY`     |
| `provider-deepseek`   | `DEEPSEEK_API_KEY`   |
| `provider-kimi`       | `MOONSHOT_API_KEY`   |
| `provider-xai`        | `XAI_API_KEY`        |
| `provider-zai`        | `ZAI_API_KEY`        |
| `provider-openrouter` | `OPENROUTER_API_KEY` |
| `provider-llamacpp`   | `LLAMACPP_API_KEY`   |

Three providers authenticate without an API Key. These are experimental.

| Provider                  | How it authenticates                                                                                                                                                                                      |
| ------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `provider-claude-code`    | Reads `~/.claude/.credentials.json`, written by the Claude Code CLI when you sign in there                                                                                                                |
| `provider-openai-codex`   | Reads `~/.codex/auth.json`, written by the Codex CLI when you sign in there                                                                                                                               |
| `provider-github-copilot` | A GitHub device flow. Call `iii trigger provider::github-copilot::login::start`, enter the `user_code` it returns at the verification URL, then call `iii trigger provider::github-copilot::login::poll`. |
