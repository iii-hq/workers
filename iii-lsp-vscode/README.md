# III Language Server - VS Code Extension

Autocompletion, hover documentation, and diagnostics for [III engine](https://github.com/iii-hq/iii) functions and triggers.

## Supported Languages

- TypeScript / TSX
- Python
- Rust

## Prerequisites

1. **iii-lsp binary** - The extension downloads and installs the pinned `iii-lsp/v0.1.0` binary on first activation.

   The binary is stored under VS Code's extension global storage directory and the absolute path is saved to `iii-lsp.serverPath` in global settings.

   To use a custom binary instead, set `iii-lsp.serverPath` to an existing executable path before activation.

2. **III engine** running locally (default: `ws://127.0.0.1:49134`)

## Installation

After the extension is installed, open a supported TypeScript, TSX, Python, or Rust file to activate it. On first activation, the extension downloads the matching `iii-lsp/v0.1.0` binary, verifies its SHA-256 checksum, installs it under extension global storage, and saves the installed path to `iii-lsp.serverPath`.

If automatic install fails, the extension warns and falls back to the configured `iii-lsp.serverPath` or `iii-lsp` on `PATH`.

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

### Local smoke test with Cursor

Build a local VSIX and install it in Cursor:

```bash
cd iii-lsp-vscode
make build
cursor --install-extension iii-lsp.vsix --force
```

Open a supported file to activate the extension:

```bash
cursor ../iii-lsp/src/main.rs
```

After activation, `iii-lsp.serverPath` should point to the downloaded binary in Cursor settings. To install the same VSIX in VS Code instead, replace `cursor` with `code`.

## Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `iii-lsp.serverPath` | `""` (auto-filled after first activation) | Path to the installed or custom `iii-lsp` binary |
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
