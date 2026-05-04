# skills

Agentic content registry worker for the [iii engine](https://github.com/iii-hq/iii).
Persists two kinds of content for AI clients: **skills** (markdown
orientation docs about a worker's tools, served at `iii://{id}` plus
an auto-rendered `iii://skills` index) and **prompts** (parametric
slash-command templates, dispatched on demand to a registered handler
function). Workers register their content at boot via `iii.trigger`;
skills persists everything to iii-state and emits change
notifications on `skills::on-change` / `prompts::on-change` for any
other worker that wants to react.

| Surface | What clients see | When to use it |
|---|---|---|
| **Skills** | Markdown documents under `iii://{id}` plus an `iii://skills` index | Orientation: "when and why to use my worker's tools" |
| **Prompts** | Slash-commands (e.g. `/send-email`) with declared arguments | Parametric command templates the *user* invokes |

The rest of this README walks you through installing the worker and
writing a worker that publishes skills and slash-command prompts.

---

## Table of contents

1. [Install](#install)
2. [Quickstart: publish a skill and a slash-command](#quickstart-publish-a-skill-and-a-slash-command)
3. [Configuration](#configuration)
4. [Functions](#functions)
5. [Custom trigger types](#custom-trigger-types)
6. [Local development & testing](#local-development--testing)

---

## Install

```bash
iii worker add skills
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next
`iii start`.

---

## Quickstart: publish a skill and a slash-command

A worker can plug into this registry three ways. They compose: most
workers ship one top-level skill, optional sub-skill markdown sections,
and zero or more slash-commands.

The interface is two `iii.trigger` calls, plus optional in-process
handler functions for sub-skills and prompt rendering:

| Step | Function id | What it stores |
|---|---|---|
| Register a skill | `skills::register` | Markdown body keyed by id |
| Register a prompt | `prompts::register` | Slash-command name + args + handler function id |
| Subscribe to changes | `skills::on-change` / `prompts::on-change` | Custom trigger types fired on every mutation |

### Skills (markdown orientation docs)

A **skill** is a markdown document explaining when and why to use your
worker's tools. Skills don't replicate the JSON schemas of your
functions — clients get those from the engine's tool listing. Skills
tell the LLM *which tool for which job*.

Register one with `skills::register`:

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

let iii = register_worker("ws://localhost:49134", InitOptions::default());

iii.trigger(TriggerRequest {
    function_id: "skills::register".into(),
    payload: json!({
        "id": "myworker",
        "skill": include_str!("../docs/skill.md"),
    }),
    action: None,
    timeout_ms: Some(5_000),
}).await?;
```

Validation rules (rejected at registration time):

- `id`: lowercase ASCII letters, digits, `-` and `_` only; max 64 chars
- `skill`: non-empty; max 256 KiB

Re-registering with the same id overwrites the body and refreshes the
`registered_at` timestamp. **Workers MUST re-register on every boot**
so any updated content shipped with the worker lands in the registry.
Doing this unconditionally also keeps you robust to state being wiped
during local development.

#### URI scheme

| URI | Returns |
|---|---|
| `iii://skills` | Auto-rendered markdown index of every registered skill (entry point) |
| `iii://{your_id}` | The body you registered |
| `iii://{your_id}/{your_function_id}` | Triggers `your_function_id` with `{}` and returns its output as content |

The third shape lets you split a skill across multiple files. Register
your top-level skill at `iii://{your_id}`, then reference sub-content
inside it as markdown links to functions you also registered. Any
function the engine knows about is reachable this way; there's no
opt-in flag. A short reserved-prefix list (engine internals, state
plumbing, this worker's own admin functions) is rejected at read time
to keep the resolver from tunneling back into infra.

A function backing `iii://{id}/{fn}` should return one of:

- a string → served as `text/markdown`
- `{ "content": "..." }` → the `content` field is served as `text/markdown`
- anything else → pretty-printed JSON, served as `application/json`

The auto-rendered `iii://skills` index uses the first H1 of each skill
as the link title and the first non-heading paragraph as the
description (truncated at 140 chars). Lead with a `# {title}` and a
short summary paragraph and the index reads cleanly without further
work.

#### Worked example: skill with sub-skills

```rust
use iii_sdk::{RegisterFunction, TriggerRequest};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Serialize, JsonSchema)]
struct SkillContent {
    content: String,
}

iii.trigger(TriggerRequest {
    function_id: "skills::register".into(),
    payload: json!({
        "id": "brain",
        "skill": "# brain\n\nHelps build UIs with Tailwind.\n\n\
                  See [`summarize`](iii://brain/brain::summarize) \
                  for the catalogue.\n",
    }),
    action: None,
    timeout_ms: Some(5_000),
}).await?;

iii.register_function(
    RegisterFunction::new("brain::summarize", |_input: Value| {
        Ok::<_, String>(SkillContent {
            content: include_str!("../docs/summarize.md").to_string(),
        })
    })
    .description("Index of UI design guidelines."),
);
```

Now any consumer can read `iii://brain` for the orientation and click
through to `iii://brain/brain::summarize` for the catalogue. The
resource resolver invokes the sub-skill function directly through
`iii.trigger`, so it doesn't need any extra opt-in flag.

### Prompts (slash commands)

A **prompt** is a parametric template the *user* invokes from the
client (e.g. typing `/send-email`). The template is rendered by your
handler function with the user-supplied arguments.

Registration is two steps:

1. Register a normal handler function. No special opt-in flag is
   needed — `skills` dispatches the handler directly through
   `iii.trigger` when the prompt is invoked.
2. Register the prompt itself, pointing at that function.

```rust
use iii_sdk::{RegisterFunction, TriggerRequest};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Deserialize, JsonSchema)]
struct SendEmailArgs {
    to: String,
    subject: String,
}

#[derive(Serialize, JsonSchema)]
struct PromptOutput {
    content: String,
}

// Step 1: handler renders the prompt
iii.register_function(
    RegisterFunction::new("myworker::send_email_prompt", |args: SendEmailArgs| {
        Ok::<_, String>(PromptOutput {
            content: format!(
                "Compose an email to {} with the subject \"{}\". Be concise and friendly.",
                args.to, args.subject
            ),
        })
    })
    .description("Render the send-email slash-command body."),
);

// Step 2: register the slash-command
iii.trigger(TriggerRequest {
    function_id: "prompts::register".into(),
    payload: json!({
        "name": "send-email",
        "description": "Compose and send an email",
        "arguments": [
            { "name": "to",      "description": "Recipient address", "required": true },
            { "name": "subject", "description": "Subject line",      "required": true }
        ],
        "function_id": "myworker::send_email_prompt"
    }),
    action: None,
    timeout_ms: Some(5_000),
}).await?;
```

Validation rules (rejected at registration time):

- `name`: lowercase ASCII letters, digits, `-` and `_` only; max 64
  chars (no `::`, no `/`, no whitespace)
- `description`: non-empty after trim
- `function_id`: non-empty after trim
- `arguments[].name`: non-empty, no duplicates within the list

#### Argument schema gotcha

The `arguments` field is what the *client* sees in its argument-picker
UI. **It does not auto-validate at runtime.** The handler is
responsible for validating its own input — treat the schema as a
contract you uphold inside the function.

#### Output normalization

The handler can return any of:

- `String` → wrapped as a single user-text message
- `{ "content": "..." }` → wrapped as a single user-text message
- `{ "messages": [ ... ] }` → passed through unchanged (full control;
  use this for multi-turn templates or assistant-prefilled responses)
- anything else → an error returned to the client

### Lifecycle & boot-time handshake

Skills and prompts are stored in iii-state under scopes `skills` and
`prompts` (configurable). Both registries are durable and survive
restarts of either `skills` or your worker.

Workers MUST re-register on every boot. `skills` itself can be
absent or come up *after* your worker, so treat the registration as
best-effort with capped exponential backoff:

```rust
use std::sync::Arc;
use std::time::{Duration, Instant};
use iii_sdk::{TriggerRequest, III};
use serde_json::json;

fn register_with_iii_skills(iii: Arc<III>) {
    tokio::spawn(async move {
        let mut backoff = Duration::from_secs(5);
        let started = Instant::now();
        loop {
            let result = iii.trigger(TriggerRequest {
                function_id: "skills::register".into(),
                payload: json!({
                    "id": "myworker",
                    "skill": include_str!("../docs/skill.md"),
                }),
                action: None,
                timeout_ms: Some(5_000),
            }).await;
            if result.is_ok() {
                tracing::info!("registered myworker skill");
                return;
            }
            if started.elapsed() > Duration::from_secs(180) {
                tracing::warn!("skills handshake gave up; install / start it and restart this worker");
                return;
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(60));
        }
    });
}
```

On graceful shutdown, optionally call `skills::unregister` and
`prompts::unregister` to clean up. Crashes leave entries in state
indefinitely; an operator can list them with `skills::list` /
`prompts::list` (which expose `registered_at`) and remove dead ones
manually.

Mutations of either scope fire the `skills::on-change` /
`prompts::on-change` custom trigger types automatically. Any worker
can subscribe and react to registrations without polling.

### Subscribing to registry changes

Workers that need to react to registrations (a dashboard, a metrics
sink, a sibling protocol bridge) subscribe to the custom trigger
types:

```rust
use iii_sdk::RegisterTriggerInput;
use serde_json::json;

iii.register_trigger(RegisterTriggerInput {
    trigger_type: "skills::on-change".into(),
    function_id: "myworker::on_skill_change".into(),
    config: json!({}),
    metadata: None,
})?;
```

Payload sent to each subscriber:

| Trigger type | Payload |
|---|---|
| `skills::on-change` | `{ "op": "register" \| "unregister", "id": "<skill-id>" }` |
| `prompts::on-change` | `{ "op": "register" \| "unregister", "name": "<prompt-name>" }` |

Dispatches are fire-and-forget (Void), so the write path on
`skills::register` / `prompts::register` doesn't block on downstream
latency. Idempotent unregisters that found nothing to delete don't
fire.

### Full multi-language example

Self-contained worker that registers one skill, one sub-skill function,
and one slash-command prompt. Boots, registers everything, then sleeps
until SIGINT.

#### Rust

```rust
use iii_sdk::{register_worker, InitOptions, RegisterFunction, TriggerRequest};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Default, Deserialize, JsonSchema)]
struct GreetArgs {
    name: String,
}

#[derive(Serialize, JsonSchema)]
struct SkillContent {
    content: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    // 1. Sub-skill function: returns extended docs as markdown.
    iii.register_function(
        RegisterFunction::new("demo::guide", |_input: Value| {
            Ok::<_, String>(SkillContent {
                content: "# Demo guide\n\nThe long-form docs for the demo worker."
                    .into(),
            })
        })
        .description("Detailed guide for the demo worker."),
    );

    // 2. Prompt handler: turns slash-command args into a templated message.
    iii.register_function(
        RegisterFunction::new("demo::greet_prompt", |input: GreetArgs| {
            Ok::<_, String>(SkillContent {
                content: format!("Greet {} warmly.", input.name),
            })
        })
        .description("Render the greet prompt."),
    );

    // Wire the registries. Always skills::register / prompts::register
    // at startup so the entries refresh after any worker upgrade.
    iii.trigger(TriggerRequest {
        function_id: "skills::register".into(),
        payload: json!({
            "id": "demo",
            "skill": "# demo\n\n\
                      A demo worker.\n\n\
                      See [`guide`](iii://demo/demo::guide) for the long version.\n"
        }),
        action: None,
        timeout_ms: Some(5_000),
    }).await?;

    iii.trigger(TriggerRequest {
        function_id: "prompts::register".into(),
        payload: json!({
            "name": "greet",
            "description": "Compose a greeting.",
            "arguments": [
                { "name": "name", "description": "Who to greet", "required": true }
            ],
            "function_id": "demo::greet_prompt"
        }),
        action: None,
        timeout_ms: Some(5_000),
    }).await?;

    tokio::signal::ctrl_c().await?;
    Ok(())
}
```

#### Node

```js
import { registerWorker } from 'iii-sdk'

const iii = registerWorker('ws://localhost:49134')

iii.registerFunction({ id: 'demo::guide' }, async () => ({
  content: '# Demo guide\n\nThe long-form docs for the demo worker.',
}))

iii.registerFunction({ id: 'demo::greet_prompt' }, async ({ name = 'world' }) => ({
  content: `Greet ${name} warmly.`,
}))

await iii.trigger({
  function_id: 'skills::register',
  payload: {
    id: 'demo',
    skill: '# demo\n\nA demo worker.\n\nSee [`guide`](iii://demo/demo::guide) for the long version.\n',
  },
})

await iii.trigger({
  function_id: 'prompts::register',
  payload: {
    name: 'greet',
    description: 'Compose a greeting.',
    arguments: [{ name: 'name', description: 'Who to greet', required: true }],
    function_id: 'demo::greet_prompt',
  },
})
```

#### Python

```python
from iii_sdk import register_worker

iii = register_worker('ws://localhost:49134')

@iii.register_function('demo::guide')
async def guide(_input):
    return {'content': '# Demo guide\n\nThe long-form docs for the demo worker.'}

@iii.register_function('demo::greet_prompt')
async def greet_prompt(input):
    name = input.get('name', 'world')
    return {'content': f'Greet {name} warmly.'}

await iii.trigger(
    function_id='skills::register',
    payload={
        'id': 'demo',
        'skill': '# demo\n\nA demo worker.\n\nSee [`guide`](iii://demo/demo::guide) for the long version.\n',
    },
)

await iii.trigger(
    function_id='prompts::register',
    payload={
        'name': 'greet',
        'description': 'Compose a greeting.',
        'arguments': [{'name': 'name', 'description': 'Who to greet', 'required': True}],
        'function_id': 'demo::greet_prompt',
    },
)
```

---

## Configuration

```yaml
# skills runtime config.

# State scopes used to persist the two registries. Changing these at
# runtime is supported but orphans prior entries; treat them as
# deployment-time constants in practice.
scopes:
  skills: skills
  prompts: prompts

# Default timeout for state::* and sub-skill function triggers (ms).
state_timeout_ms: 10000
```

CLI flags:

```text
--config <PATH>    Path to config.yaml [default: ./config.yaml]
--url <URL>        WebSocket URL of the iii engine [default: ws://127.0.0.1:49134]
--manifest         Output the module manifest as JSON and exit
-h, --help         Print help
```

If the config file is missing or malformed the worker logs a warning
and falls back to the defaults — boot is never blocked by a bad
config path.

---

## Functions

Eleven functions across the two registries. The six public CRUD
entries are callable by any worker over `iii.trigger`. The five
internal-RPC entries are reserved for protocol-bridge workers that
serve the registries to external clients; they never surface as agent
tools.

| Function ID | Description |
|---|---|
| `skills::register` | Store a markdown skill body keyed by id. |
| `skills::unregister` | Delete a skill by id. Idempotent. |
| `skills::list` | Metadata-only listing, sorted by id. |
| `skills::resources-list` | Internal: enumerate registered skills as resource entries. |
| `skills::resources-read` | Internal: resolve an `iii://` URI to its content. |
| `skills::resources-templates` | Internal: declare the `iii://{id}` URI templates. |
| `prompts::register` | Store a slash-command prompt definition. |
| `prompts::unregister` | Delete a prompt by name. Idempotent. |
| `prompts::list` | Metadata-only listing, sorted by name. |
| `prompts::mcp-list` | Internal: enumerate registered prompts with argument schemas. |
| `prompts::mcp-get` | Internal: dispatch a registered prompt handler with caller-supplied arguments. |

---

## Custom trigger types

| Trigger type | Fires when | Payload to subscribers |
|---|---|---|
| `skills::on-change` | After every mutation of the skills registry | `{ "op": "register" \| "unregister", "id": "<skill-id>" }` |
| `prompts::on-change` | After every mutation of the prompts registry | `{ "op": "register" \| "unregister", "name": "<prompt-name>" }` |

---

## Local development & testing

### Run from source

```bash
cargo run --release -- --url ws://127.0.0.1:49134 --config ./config.yaml
```

### Tests

```bash
# Fast, offline — exercises the pure helpers (markdown / URI / validators)
# without needing an iii engine.
cargo test --test bdd -- --tags @pure

# Full suite — requires an iii engine on ws://127.0.0.1:49134
# (or III_ENGINE_WS_URL). Runs the full BDD scenario set covering
# every function and every validation rule.
cargo test

# One feature group at a time. Available tags:
#   @pure  @markdown
#   @engine  @skills_register  @skills_resources
#   @prompts_register  @prompts_get  @notifications
cargo test --test bdd -- --tags @skills_resources
```

The BDD harness lives under [tests/](tests/). Feature files mirror the
modules in [src/functions/](src/functions/). Step definitions under
[tests/steps/](tests/steps/) drive each feature through the same
`iii.trigger` path the production binary uses.
