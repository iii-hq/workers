const { workspace } = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

let client;

function activate(context) {
  const config = workspace.getConfiguration("iii-lsp");
  const serverPath = config.get("serverPath") || "iii-lsp";
  const engineUrl = config.get("engineUrl") || "ws://127.0.0.1:49134";

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
