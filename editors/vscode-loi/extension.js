// Minimal language client for the Loi language server.
// Install dependencies with `npm install`, then run `npm run compile` is not
// needed: this file uses plain CommonJS. Press F5 in VS Code to try it out.
const path = require("path");
const vscode = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

let client;

function activate(context) {
  const config = vscode.workspace.getConfiguration("loi");
  const workspace = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath || process.cwd();
  const configured = config.get("lsp.command", "${workspaceFolder}/loi-lsp");
  const command = configured.replace("${workspaceFolder}", workspace);
  const compiler = config.get("compiler", "loi");

  const serverOptions = {
    command: path.resolve(workspace, command),
    args: [],
    options: { env: { ...process.env, LOI_COMPILER: compiler } },
    transport: TransportKind.stdio,
  };
  const clientOptions = {
    documentSelector: [{ scheme: "file", language: "loi" }],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher("**/*.loi"),
    },
  };

  client = new LanguageClient("loi", "Loi Language Server", serverOptions, clientOptions);
  context.subscriptions.push(client.start());
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
