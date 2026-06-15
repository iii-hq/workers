const vscode = require("vscode");
const fs = require("node:fs");
const path = require("node:path");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

const { ensureServerBinary } = require("./installer");

let client;

// Resolve the server binary on PATH, preferring the new `lsp` name and
// falling back to the legacy `iii-lsp` name for older installs.
function resolvePathBinary() {
  const candidates = ["lsp", "iii-lsp"];
  const pathEntries = (process.env.PATH || "").split(path.delimiter).filter(Boolean);
  const exts = process.platform === "win32"
    ? (process.env.PATHEXT || ".EXE;.CMD;.BAT").split(";")
    : [""];
  for (const candidate of candidates) {
    for (const dir of pathEntries) {
      for (const ext of exts) {
        const full = path.join(dir, candidate + ext);
        try {
          if (fs.statSync(full).isFile()) {
            return full;
          }
        } catch {
          // not here, keep looking
        }
      }
    }
  }
  // Nothing on PATH; default to the new name and let the OS resolve it.
  return candidates[0];
}

async function activate(context) {
  const config = vscode.workspace.getConfiguration("iii-lsp");
  const engineUrl = config.get("engineUrl") || "ws://127.0.0.1:49134";
  let serverPath;

  try {
    serverPath = await ensureServerBinary(context, vscode);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    vscode.window.showWarningMessage(
      `Failed to install iii-lsp binary. Falling back to configured path or PATH lookup. ${message}`
    );
    serverPath = config.get("serverPath") || resolvePathBinary();
  }

  const serverOptions = {
    run: {
      command: serverPath,
      args: ["--url", engineUrl],
      transport: TransportKind.stdio,
    },
    debug: {
      command: serverPath,
      args: ["--url", engineUrl],
      transport: TransportKind.stdio,
    },
  };

  const clientOptions = {
    documentSelector: [
      { scheme: "file", language: "typescript" },
      { scheme: "file", language: "typescriptreact" },
      { scheme: "file", language: "python" },
      { scheme: "file", language: "rust" },
    ],
  };

  client = new LanguageClient(
    "iii-lsp",
    "iii Language Server",
    serverOptions,
    clientOptions
  );

  client.start();
}

function deactivate() {
  if (client) {
    return client.stop();
  }
}

module.exports = { activate, deactivate };
