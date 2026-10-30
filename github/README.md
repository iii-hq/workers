## Monitoramento durável de PR (opt-in)

Além das APIs CLI existentes, `github::pr::watch`, `github::pr::unwatch`,
`github::pr::watch-status`, `github::pr::recover` e a função observável
`github::pr::on-event` implementam monitoramento por webhooks. O recurso fica desabilitado por padrão;
consulte o [guia de webhooks](../docs/architecture/github-webhooks.md) para
conhecer as permissões, a recuperação e as limitações. Nenhum recurso externo é
criado ao registrar a interface com a configuração padrão.

Antes de habilitá-lo, abra **GitHub → Webhooks** no Console ou chame
`github::setup::webhooks-status`. A lista verifica se o worker opcional
`quick-tunnel` está instalado, se o `cloudflared` está disponível e se o
listener restrito do `http` foi realmente aberto. O Console só instala
`quick-tunnel` depois da confirmação do operador; ele nunca instala
`cloudflared` e, quando necessário, mostra o
[guia oficial de instalação](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/).
Use `github::setup::enable-http-listener` e
`github::setup::enable-webhooks` para as alterações guiadas. Ativar os webhooks
exige todos os itens confirmados e requer reiniciar o worker `github` para
abrir o armazenamento. `github::pr::watch` recusa apenas pré-requisitos
sabidamente ausentes ou bloqueados; uma verificação temporariamente
`unknown` não gera um falso bloqueio.

# github

GitHub as iii functions, powered by the GitHub CLI. Typed `github::*`
functions cover pull requests, issues, repos, Actions runs and workflows,
releases, and search; `github::exec` runs any other gh command and
`github::api` reaches any GitHub REST endpoint. Agents get
schema-discoverable GitHub operations with read-vs-mutate permission gating
instead of raw shell.

## Install

```bash
iii call compose::add worker=github
```

`iii call compose::add` resolves the worker and its dependencies, writes
exact declarations to `worker-compose.yaml`, and reconciles the Compose project.

The worker shells out to the [GitHub CLI](https://cli.github.com) — install
a current `gh` (the typed field sets are validated against gh 2.94) and give
it credentials: either `gh auth login` on the host, or set `GH_TOKEN` for
the worker (see Configuration).

## Quickstart

```rust
use iii_sdk::{register_worker, CallRequest, InitOptions};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    // Open PRs in a repo, typed and parsed:
    let prs = iii
        .call(CallRequest {
            function_id: "github::pr::list".into(),
            payload: json!({ "repo": "cli/cli", "state": "open", "limit": 5 }),
            action: None,
            timeout_ms: Some(60_000),
        })
        .await?;
    println!("{prs:#?}"); // { value: [{ number, title, state, url, … }] }

    // Anything else gh can do, verbatim:
    let version = iii
        .call(CallRequest {
            function_id: "github::exec".into(),
            payload: json!({ "args": ["--version"] }),
            action: None,
            timeout_ms: Some(60_000),
        })
        .await?;
    println!("{version:#?}"); // { stdout, stderr, exit_code, … }

    Ok(())
}
```

## Observable functions

| Function | Args | Delivery | Event |
|---|---|---|---|
| `github::on-called` | none; filter on `input.function_id`, `input.repo`, `input.ok` | iii delivers each call to every observer whose filters match, fire-and-forget | `{ function_id, args_summary, repo, ok, duration_ms, result_summary, kind, result_preview, timestamp }` |
| `github::pr::on-event` | `{ notifications?, watch_id?, repo?, number?, categories? }` | Custom: each notification goes to the one observer it is for, and its target's result acks it | the full event (`PrEvent`), or a compact event or digest per the observer's notification policy |

Both functions are private: only this worker calls them, and any worker may
observe them. Each handler returns the event it is called with.

**`github::on-called`.** The worker calls it after every `github::*` function
runs, only while something observes it. The event carries the call that ran
and a short, bounded result summary; the console's GitHub page observes it for
its activity feed and live git graph.

```typescript
iii.observe(
  { function_id: 'github::on-called', filters: { 'input.ok': { $eq: false } }, deliver: 'output' },
  'alerts::github-failure',
)
```

**`github::pr::on-event`.** Durable PR lifecycle, comment, review and CI
notifications for the watches started with `github::pr::watch`. Observe it
before calling `github::pr::watch`, then read `github::pr::watch-status` once.
An observer's args select what it receives:

- `watch_id`, `repo` (case-insensitive), `number`: the watches it listens to;
  omitted fields match every watch.
- `categories`: any of `ci`, `comments`, `reviews`, `pr`; omitted means all.
- `notifications`: its notification policy (`profile: all | agent_actionable |
  review_assistant`, `success_checks`, `ignored_actors`, `quiet_ms`,
  `max_wait_ms`, `max_items`, …); omitted inherits `webhooks.notifications`.

The worker persists each observer as a subscriber when it is added, and the
watches it listens to stay alive while it exists (see
[Watch lifetime](skills/SKILL.md)). Delivery is durable and at-least-once:
each notification is a job in the worker's SQLite outbox, called as one call
of `github::pr::on-event` with that observer's payload (`profile: all` gets
the full event, the other profiles a compact event or a digest). The target
receives the payload itself, and the observer's `deliver` choice does not
apply. A target that returns acks the notification; an error or a timeout
(8 s) keeps the job and retries it. A notification the observer's filters
reject is acked without a call. Targets dedupe on `event_id`.

```rust
use iii_sdk::{ObserverOptions, Source};
use serde_json::json;

iii.observe_with(
    Source::function("github::pr::on-event").args(json!({
        "watch_id": "pr-1234",
        "notifications": { "profile": "agent_actionable" },
    })),
    "agent::on-pr-event",
    ObserverOptions::default().metadata(json!({ "session_id": "s-1" })),
);
```

An observer is rejected with `INVALID_ARGS` when its `repo` or notification
policy is invalid, and with `STORAGE_FAILURE` when the webhook store cannot
persist it.

The worker observes, for its own plumbing:

| Function | Args or filters | Target |
|---|---|---|
| `configuration::on-updated` | filters `input.id` = this worker's entry, `input.event_type` = `configuration:updated` | `github::on-config-change` |
| `http::on-request` | args `{ method: POST, path: /webhooks/github/:endpoint_id, public_webhook: true }` | `github::webhooks::receive` |
| `queue::on-message` | args `{ topic: webhooks.queue, group: github-webhooks, max_retries: 5, backoff_ms: 1000 }`, observer id `<worker name>:webhooks:<webhooks.queue>` | `github::webhooks::process` |
| `quick-tunnel::on-changed` | filter `input.tunnel_id` = `webhooks.tunnel_id` | `github::webhooks::tunnel-changed` |
| `cron::on-tick` | args `{ expression: "*/10 * * * * *" }` | `github::webhooks::maintain` |

The four webhook observers exist only when `webhooks.enabled` is true. Each
stays pending while its function's worker is not running.

The job queue is observed as the consumer group `github-webhooks`, whose
durable queue keeps the jobs nobody has acked, under a stable observer id:
the worker name (`github`, or the `III_WORKER_NAME` its supervisor sets), then
`:webhooks:` and the queue, preceded by `<namespace>/` when the worker has a
namespace (`github:webhooks:github-webhooks` by default; an id over 128 bytes
keeps a SHA-256 digest of it). When the worker stops, killed or not, the engine
keeps its observers suspended for its grace period
(`observers.observer_grace_ms`, 30 s by default) and withdraws them from the
queue and http workers, which frees the webhook route. A restarted worker,
under the same name and namespace, observes the queue under the same id: the
engine replaces the suspended observer with the new one, and the queue worker
hands the new process every job the previous one left unacked, once.

A refusal because another observer still holds the route
(`HTTP_ROUTE_CONFLICT`) or the queue observer's id (`OBSERVER_ID_CONFLICT`:
the engine has not seen the previous process's connection close yet, or
another worker holds the id until its grace period ends) is recorded in the
webhook health's `last_error`; the worker observes again after 1 s, then twice
as long each time up to 30 s, and clears the error once the observer is
active.

## Configuration

```yaml
gh_executable: ""            # path to gh; empty = `gh` on PATH
token: "${GH_TOKEN}"         # env-expanded; empty = ambient `gh auth login`
default_timeout_ms: 30000    # per-call timeout when timeout_ms is omitted
max_timeout_ms: 120000       # upper clamp for any per-call timeout_ms
max_output_bytes: 1048576    # per-stream capture cap (flags *_truncated)
```

Other keys (and their defaults) live in [`src/config.rs`](src/config.rs).
