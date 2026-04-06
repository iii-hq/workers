# III Language Server - VS Code Extension

Autocompletion, hover documentation, and diagnostics for [III engine](https://github.com/iii-org) functions and triggers.

## Supported Languages

- TypeScript / TSX
- Python
- Rust

## Prerequisites

1. **iii-lsp binary** - Build it from the `iii-lsp/` crate:

   ```bash
   cd iii-lsp
   cargo build --release
   ```

   Then either:
   - Add `iii-lsp/target/release` to your `PATH`, or
   - Set the binary path in the extension settings (see below)

2. **III engine** running locally (default: `ws://127.0.0.1:49134`)

## Installation

### From source (development)

1. Install dependencies:

   ```bash
   cd iii-lsp-vscode
   npm install
   ```

2. Open VS Code and install the extension locally:

   ```bash
   # From the repo root
   code --install-extension iii-lsp-vscode
   ```

   Or, in VS Code:
   - Open the Command Palette (`Cmd+Shift+P`)
   - Run **Extensions: Install from Location...**
   - Select the `iii-lsp-vscode` directory

### From VSIX package

1. Package the extension:

   ```bash
   cd iii-lsp-vscode
   npm install
   npx @vscode/vsce package
   ```

2. Install the `.vsix` file:

   ```bash
   code --install-extension iii-lsp-*.vsix
   ```

## Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `iii-lsp.serverPath` | `""` (uses `iii-lsp` from PATH) | Path to the `iii-lsp` binary |
| `iii-lsp.engineUrl` | `ws://127.0.0.1:49134` | WebSocket URL of the III engine |

Configure via **Settings** > search "III LSP", or in `settings.json`:

```json
{
  "iii-lsp.serverPath": "/path/to/iii-lsp",
  "iii-lsp.engineUrl": "ws://127.0.0.1:49134"
}
```

## Features

- **Completions** - Function IDs, trigger types, payload properties, trigger config properties, and known values (stream names, topics, API paths)
- **Hover** - Function documentation with request/response JSON schemas
- **Diagnostics** - Validates function IDs, required payload fields, trigger types, config properties, cron expressions, and HTTP methods
