// VS Code extension entry point: starts `bailiwick lsp` over stdio for .bwk
// files. Syntax highlighting comes from the TextMate grammar in syntaxes/
// and needs no server.
//
// The server ships inside the extension (server/bailiwick, or
// server/bailiwick.exe on Windows), built for each platform. A user who sets
// `bwk.serverPath` gets their own binary instead.
const fs = require("fs");
const path = require("path");
const vscode = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

let client;

/** The server to run: the user's setting, else the bundled binary. */
function serverCommand(extensionPath, configured, platform = process.platform) {
  if (configured && configured !== "bailiwick") return configured;
  const bundled = path.join(extensionPath, "server", platform === "win32" ? "bailiwick.exe" : "bailiwick");
  if (fs.existsSync(bundled)) return bundled;
  // Not bundled for this platform: fall back to `bailiwick` on the PATH.
  return "bailiwick";
}

function activate(context) {
  const configured = vscode.workspace.getConfiguration("bwk").get("serverPath", "bailiwick");
  const command = serverCommand(context.extensionPath, configured);
  const serverOptions = { command, args: ["lsp"], transport: TransportKind.stdio };
  const clientOptions = {
    documentSelector: [{ scheme: "file", language: "bwk" }, { scheme: "untitled", language: "bwk" }],
  };
  client = new LanguageClient("bwk", "Bailiwick", serverOptions, clientOptions);
  client.start().catch((err) => {
    vscode.window.showWarningMessage(
      `Bailiwick: the language server could not start (${err && err.message ? err.message : err}). ` +
        "Highlighting still works. Set `bwk.serverPath` to a `bailiwick` binary to enable checking, completion and hover."
    );
  });
  context.subscriptions.push({ dispose: () => client && client.stop() });
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate, serverCommand };
