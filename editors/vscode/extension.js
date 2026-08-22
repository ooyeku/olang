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

// Resolve the server binary: the explicit setting first, then PATH, then
// the well-known install locations. A Dock-launched editor gets launchd's
// minimal PATH — /usr/bin:/bin:… — which contains none of the places
// olang actually installs to, so "just use PATH" is exactly the silent
// failure this extension used to have.
const os = require("os");
const path = require("path");

const WELL_KNOWN = [
  path.join(os.homedir(), ".olang", "olang"),
  path.join(os.homedir(), ".cargo", "bin", "olang"),
  "/usr/local/bin/olang",
  "/opt/homebrew/bin/olang",
];

async function resolveServer(configured) {
  if (configured !== "olang") {
    return { path: configured, version: await probeServer(configured) };
  }
  const onPath = await probeServer("olang");
  if (onPath !== null) {
    return { path: "olang", version: onPath };
  }
  for (const candidate of WELL_KNOWN) {
    const v = await probeServer(candidate);
    if (v !== null) {
      return { path: candidate, version: v };
    }
  }
  return { path: configured, version: null };
}

async function activate() {
  const configured = workspace.getConfiguration("olang").get("serverPath", "olang");
  const { path: serverPath, version } = await resolveServer(configured);
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
