// Language client for the Loi language server.
// `loi-lsp` is a separate stdio executable, like clangd. Install dependencies
// with `npm install`, then press F5 in VS Code to run an Extension Development
// Host. Plain CommonJS keeps the extension build-free.
const fs = require("fs");
const os = require("os");
const path = require("path");
const vscode = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

let client;

function expandVariables(value, workspace) {
  if (typeof value !== "string") {
    return value;
  }
  return value
    .replaceAll("${workspaceFolder}", workspace)
    .replaceAll("${workspaceFolderBasename}", path.basename(workspace))
    .replaceAll("${userHome}", os.homedir());
}

function isFile(candidate) {
  try {
    return fs.statSync(candidate).isFile();
  } catch {
    return false;
  }
}

function fileCandidates(base) {
  if (process.platform !== "win32" || path.extname(base)) {
    return [base];
  }
  const extensions = (process.env.PATHEXT || ".COM;.EXE;.BAT;.CMD")
    .split(";")
    .map((extension) => extension.trim())
    .filter(Boolean);
  return extensions.map((extension) => base + extension);
}

function firstExisting(candidates) {
  for (const candidate of candidates) {
    if (isFile(candidate)) {
      return candidate;
    }
  }
  return null;
}

function findOnPath(command) {
  for (const directory of (process.env.PATH || "").split(path.delimiter)) {
    if (!directory) {
      continue;
    }
    const found = firstExisting(fileCandidates(path.join(directory, command)));
    if (found) {
      return found;
    }
  }
  return null;
}

function workspaceDirectories(workspace) {
  return [
    path.join(workspace, "target", "debug"),
    path.join(workspace, "target", "release"),
    workspace,
  ];
}

function resolveCommand(command, workspace, extraDirectories = []) {
  const expanded = expandVariables(command, workspace);
  if (path.isAbsolute(expanded) || expanded.includes("/") || expanded.includes("\\")) {
    const base = path.resolve(workspace, expanded);
    return firstExisting(fileCandidates(base)) || base;
  }
  const onPath = findOnPath(expanded);
  if (onPath) {
    return onPath;
  }
  for (const directory of extraDirectories) {
    const found = firstExisting(fileCandidates(path.join(directory, expanded)));
    if (found) {
      return found;
    }
  }
  return expanded;
}

/// A bare compiler name should come from the directory of the server we just
/// found; otherwise a different `loi` earlier on PATH could be paired with it.
function resolveCompiler(command, workspace, serverCommand) {
  const expanded = expandVariables(command, workspace);
  if (path.isAbsolute(expanded) || expanded.includes("/") || expanded.includes("\\")) {
    return resolveCommand(expanded, workspace);
  }
  const sibling = firstExisting(
    fileCandidates(path.join(path.dirname(serverCommand), expanded))
  );
  if (sibling) {
    return sibling;
  }
  return findOnPath(expanded) || expanded;
}

function cacheDirectory(config, workspace) {
  const configured = expandVariables(config.get("lsp.cacheDir", ""), workspace);
  const directory = configured || path.join(os.tmpdir(), "loi-lsp-vscode");
  fs.mkdirSync(directory, { recursive: true });
  return directory;
}

function reportMissingServer(command) {
  const message =
    `Loi language server not found: ${command}. ` +
    "Build it with `cargo build` and add `target/debug` to PATH, " +
    "or set loi.lsp.command to the absolute path of loi-lsp.";
  vscode.window
    .showErrorMessage(message, "Open Settings")
    .then((choice) => {
      if (choice === "Open Settings") {
        vscode.commands.executeCommand("workbench.action.openSettings", "loi.lsp.command");
      }
    });
}

function activate(context) {
  const config = vscode.workspace.getConfiguration("loi");
  const workspace = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath || process.cwd();
  const directories = workspaceDirectories(workspace);
  const command = resolveCommand(config.get("lsp.command", "loi-lsp"), workspace, directories);

  if (!isFile(command)) {
    reportMissingServer(command);
    return;
  }

  const compilerSetting = config.get("compiler", "loi");
  const compiler = resolveCompiler(compilerSetting, workspace, command);
  const env = {
    ...process.env,
    LOI_LSP_CACHE_DIR: cacheDirectory(config, workspace),
  };
  if (compilerSetting !== "loi" || path.isAbsolute(compiler)) {
    env.LOI_COMPILER = compiler;
  } else {
    // Let the launcher pick the compiler next to the server, or the one
    // already present in LOI_COMPILER, instead of forcing a bare `loi`.
    delete env.LOI_COMPILER;
  }

  const serverOptions = {
    command,
    args: [],
    options: {
      cwd: workspace,
      env,
    },
    transport: TransportKind.stdio,
  };
  const clientOptions = {
    documentSelector: [{ scheme: "file", language: "loi" }],
    outputChannelName: "Loi Language Server",
    synchronize: {
      configurationSection: "loi",
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
