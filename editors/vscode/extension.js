// olang VS Code extension: thin client over `olang lsp`.
//
// The one thing a thin client must not do is fail silently: when the
// server binary is missing (a GUI-launched VS Code often has a shorter
// PATH than the user's shell), the old client called start() and nothing
// visibly happened — the textbook "the LSP doesn't work". The binary is
// now probed first, and failure produces an actionable message naming
// the setting to fix.
const { workspace, window } = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");
const { execFile } = require("child_process");

let client;

function probeServer(serverPath) {
  return new Promise((resolve) => {
    execFile(serverPath, ["--version"], { timeout: 5000 }, (err, stdout) => {
      resolve(err ? null : String(stdout).trim());
    });
  });
}

async function activate() {
  const serverPath = workspace.getConfiguration("olang").get("serverPath", "olang");
  const version = await probeServer(serverPath);
  if (version === null) {
    const pick = await window.showErrorMessage(
      `olang language server: cannot run '${serverPath}'. Install olang and/or set ` +
        `"olang.serverPath" to the binary's full path (GUI-launched editors often ` +
        `miss your shell's PATH).`,
      "Open Settings"
    );
    if (pick === "Open Settings") {
      const { commands } = require("vscode");
      commands.executeCommand("workbench.action.openSettings", "olang.serverPath");
    }
    return;
  }
  client = new LanguageClient(
    "olang",
    `olang language server (${version})`,
    { command: serverPath, args: ["lsp"], transport: TransportKind.stdio },
    { documentSelector: [{ scheme: "file", language: "olang" }] }
  );
  client.start();
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
