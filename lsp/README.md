# lsp

Language Server Protocol implementation for the [iii engine](https://github.com/iii-hq/iii). Provides autocompletion, hover documentation, and diagnostics for iii function calls and observers directly inside any LSP-capable editor.

The server connects to a running iii engine over WebSocket, snapshots the live registry of functions (`engine::functions::list`, with each function's `args_format`, `private`, `observable` and `observer_count`) and observers (`engine::observers::list`), and serves that catalog back to the editor as completions and hover. Because the catalog is dynamic, completions reflect whatever workers are currently connected — no hand-maintained type stubs.

## Supported languages

| Language | File extensions |
|---|---|
| TypeScript / TSX / JS / JSX | `.ts`, `.tsx`, `.js`, `.jsx` |
| Python | `.py` |
| Rust | `.rs` |

Other file types are passed through without analysis.

## Features

- **Completions** — function IDs, payload properties, observe sources and targets, observer args, and known values. Offered as you type `'`, `"`, `:`, `{`, ` `, and `=` (for Python keyword arguments).
  - `call` / `call_async` (`iii.call({ function_id, payload })`, Rust `CallRequest::new(id, payload)` and `CallRequest { function_id, .. }`): every function, then the `payload` properties of its `request_format`.
  - `observe(source, target, options?)` (TypeScript, Python, Rust `observe` / `observe_with`): the source completes observable functions and the engine sources (`engine::on-functions-changed`, `configuration::on-updated`, `observability::on-log`, `stream::on-changed`, …), in the string shorthand, as `function_id` of the source object, or in Rust `Source::function(id)`; the target completes functions that are not private (`Target::function(id)` in Rust). Inside the source's `args`, keys come from the function's `args_format` and values from its `enum` and from the args of the function's current observers (cron expressions, HTTP paths, queue topics).
  - Known values for `stream_name`, `topic` (and `queue`) and `scope`, read from the filters and args of the observers of `stream::on-*`, `publish`, `queue::on-message` and `state::on-changed`.
- **Hover** — function description, the worker that hosts it, whether it is private or not observable, its observer count and their targets, and its request, response and args JSON schemas, rendered inline. Engine sources show their description.
- **Diagnostics** — republished on every `did_open` / `did_change`:
  - calls: unknown function IDs, IDs without a namespace, required payload fields, and the deprecated aliases `trigger`, `trigger_async` and `TriggerRequest` of `call`, `call_async` and `CallRequest` (a hint tagged deprecated);
  - observers: an unknown or non-observable source, args passed to a function without `args_format`, required args missing, an unknown target, a private target, `http::on-request` args with an invalid `method`, and `cron::on-tick` args whose `expression` does not have 6 fields (sec min hour day month weekday) and an optional year.

When the engine is not reachable on startup, the server stays up and returns empty completions; once the engine comes online, fresh completions become available without restarting the editor.

## Run locally

```bash
cargo build --release
./target/release/lsp --url ws://127.0.0.1:49134
```

The binary speaks LSP over stdio; spawn it from your editor's LSP client.

### CLI flags

| Flag | Default | Description |
|---|---|---|
| `--url` (env `III_URL`) | `ws://127.0.0.1:49134` | WebSocket URL of the iii engine |
| `--stdio` | — | Accepted for editor compatibility (the server always uses stdio) |

## Editor integration

### VS Code / Cursor

Use the bundled VS Code extension, which downloads the matching `lsp` binary on first activation:

- Source: [lsp-vscode/](https://github.com/iii-hq/workers/tree/main/lsp-vscode)
- Marketplace: see [lsp-vscode/README.md](https://github.com/iii-hq/workers/blob/main/lsp-vscode/README.md) for install instructions.

### Neovim (built-in LSP)

```lua
vim.lsp.config.iii = {
  cmd = { '/path/to/lsp', '--url', 'ws://127.0.0.1:49134' },
  filetypes = { 'typescript', 'typescriptreact', 'javascript', 'python', 'rust' },
}
vim.lsp.enable('iii')
```

### Any LSP client

Configure the client to launch the `lsp` binary over stdio for the supported filetypes above. No initialization options are required.

## See also

- [lsp-vscode/README.md](https://github.com/iii-hq/workers/blob/main/lsp-vscode/README.md) — VS Code extension that wraps this binary.
- [docs/sops/new-worker.md](https://github.com/iii-hq/workers/blob/main/docs/sops/new-worker.md) — monorepo conventions and release flow.
