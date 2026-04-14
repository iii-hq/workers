const vscode = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

const { ensureServerBinary } = require("./installer");

let client;

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
    serverPath = config.get("serverPath") || "iii-lsp";
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
    "III Language Server",
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
