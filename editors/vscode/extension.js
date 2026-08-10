// olang VS Code extension: thin client over `olang lsp`.
const { workspace } = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

let client;

function activate() {
  const serverPath = workspace.getConfiguration("olang").get("serverPath", "olang");
  client = new LanguageClient(
    "olang",
    "olang language server",
    { command: serverPath, args: ["lsp"], transport: TransportKind.stdio },
    { documentSelector: [{ scheme: "file", language: "olang" }] }
  );
  client.start();
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
