# iii-lsp

Language Server Protocol implementation for the [iii engine](https://github.com/iii-hq/iii). Provides autocompletion, hover documentation, and diagnostics for iii function calls and trigger registrations directly inside any LSP-capable editor.

The server connects to a running iii engine over WebSocket, snapshots the live registry of functions and trigger types, and serves that catalog back to the editor as completions and hover. Because the catalog is dynamic, completions reflect whatever workers are currently connected — no hand-maintained type stubs.

## Supported languages

| Language | File extensions |
|---|---|
| TypeScript / TSX / JS / JSX | `.ts`, `.tsx`, `.js`, `.jsx` |
| Python | `.py` |
| Rust | `.rs` |

Other file types are passed through without analysis.

## Features

- **Completions** — function IDs, trigger types, payload properties, trigger config properties, and known values (stream names, topics, API paths). Triggered on `'`, `"`, `:`, `{`, ` `, and `=` (for Python keyword arguments).
- **Hover** — function description plus request and response JSON schemas, rendered inline.
- **Diagnostics** — validates function IDs, required payload fields, trigger types and config properties, cron expressions, and HTTP methods. Republished on every `did_open` / `did_change`.

When the engine is not reachable on startup, the server stays up and returns empty completions; once the engine comes online, fresh completions become available without restarting the editor.

## Run locally

```bash
cargo build --release
./target/release/iii-lsp --url ws://127.0.0.1:49134
```

The binary speaks LSP over stdio; spawn it from your editor's LSP client.

### CLI flags

| Flag | Default | Description |
|---|---|---|
| `--url` (env `III_URL`) | `ws://127.0.0.1:49134` | WebSocket URL of the iii engine |
| `--stdio` | — | Accepted for editor compatibility (the server always uses stdio) |

## Editor integration

### VS Code / Cursor

Use the bundled VS Code extension, which downloads the matching `iii-lsp` binary on first activation:

- Source: [iii-lsp-vscode/](../iii-lsp-vscode/)
- Marketplace: see [iii-lsp-vscode/README.md](../iii-lsp-vscode/README.md) for install instructions.

### Neovim (built-in LSP)

```lua
vim.lsp.config.iii = {
  cmd = { '/path/to/iii-lsp', '--url', 'ws://127.0.0.1:49134' },
  filetypes = { 'typescript', 'typescriptreact', 'javascript', 'python', 'rust' },
}
vim.lsp.enable('iii')
```

### Any LSP client

Configure the client to launch the `iii-lsp` binary over stdio for the supported filetypes above. No initialization options are required.

## See also

- [iii-lsp-vscode/README.md](../iii-lsp-vscode/README.md) — VS Code extension that wraps this binary.
- [AGENTS-NEW-WORKER.md](../AGENTS-NEW-WORKER.md) — monorepo conventions and release flow.
