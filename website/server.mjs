#!/usr/bin/env node
/*
 * Loi playground server.
 *
 * Serves the static website and exposes a tiny JSON API that drives the real
 * Loi compiler:
 *
 *   GET  /api/health   -> compiler availability and version
 *   POST /api/check    -> { source } -> checker result
 *   POST /api/format   -> { source } -> canonical Flow Syntax
 *   POST /api/run      -> { source } -> native output or a WebAssembly preview
 *   POST /api/run-web  -> { source, demo? } -> explicit WebAssembly preview
 *   POST /api/lsp      -> { source, changes? } -> LSP messages from the real server
 *
 * Programs are compiled and executed in a fresh temporary directory. The
 * library modules (with their C shims) and the vendored raylib bindings are
 * copied next to the source so `import "lib/math.loi"`,
 * `import "../lib/std.loi"` and `import "../vendor/raylib/raylib.loi"` resolve
 * without touching the repository.
 *
 * This is a local development tool. Executed programs run with the permissions
 * of the server process, so bind to localhost and do not expose it publicly.
 */

import http from "node:http";
import path from "node:path";
import os from "node:os";
import { existsSync, readdirSync } from "node:fs";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  copyFile,
  cp,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";

const websiteDir = path.dirname(fileURLToPath(import.meta.url));
const projectDir = path.dirname(websiteDir);
const libPath = path.join(projectDir, "lib", "std.loi");
const libDir = path.join(projectDir, "lib");
// Discover the library surface at startup so any example works without a
// hard-coded allow-list. The trusted shims are used only by the server-side LSP
// session: browser-submitted programs still cannot spawn processes or write
// arbitrary paths through the LSP document sink.
const libraryModules = readdirSync(libDir)
  .filter((name) => name.endsWith(".loi"))
  .sort();
const libraryShims = readdirSync(libDir)
  .filter((name) => name.endsWith("_shim.c"))
  .sort();
const sandboxShims = new Set(["process_shim.c", "lsp_shim.c"]);
const playgroundShims = libraryShims.filter((name) => !sandboxShims.has(name));
const raylibPath = path.join(projectDir, "vendor", "raylib", "raylib.loi");
const lspExamplePath = path.join(projectDir, "examples", "lsp_server.loi");
const webDemos = new Map([
  ["raylib_web", { caption: "Bolas, teclado e cor em WebGL." }],
  ["sound_synth", { caption: "Síntese de som via WebAudio." }],
  ["audio_visualizer", { caption: "Visualizador de áudio com sequenciador." }],
  ["text_editor", { caption: "Editor de texto com raylib." }],
]);
const webRunsDir = path.join(websiteDir, "web-runs");

const HOST = process.env.HOST || "127.0.0.1";
const PORT = Number(process.env.PORT || 4173);
const MAX_SOURCE_BYTES = 200_000;
const MAX_BODY_BYTES = MAX_SOURCE_BYTES * 2;
const MAX_OUTPUT_BYTES = 128 * 1024;
const MAX_CONCURRENT = 4;
const CHECK_TIMEOUT_MS = 25_000;
const BUILD_TIMEOUT_MS = 30_000;
const RUN_TIMEOUT_MS = 10_000;
const FORMAT_TIMEOUT_MS = 10_000;
const LSP_TIMEOUT_MS = 30_000;
const WEB_BUILD_TIMEOUT_MS = 180_000;
const WEB_RUN_TTL_MS = 30 * 60 * 1000;

function resolveCompiler() {
  if (process.env.LOI_BIN) {
    return path.resolve(process.cwd(), process.env.LOI_BIN);
  }
  const candidates = [
    path.join(projectDir, "target", "debug", "loi"),
    path.join(projectDir, "target", "release", "loi"),
  ];
  for (const candidate of candidates) {
    if (existsSync(candidate)) return candidate;
  }
  return "loi";
}

let compiler = resolveCompiler();
let activeJobs = 0;
let lspSessionPromise = null;
let webBuildSupport = null;

/* --------------------------------------------------------------------------
   Generic example routing and LSP framing
   -------------------------------------------------------------------------- */

function detectWebDemo(source) {
  if (source.includes("loi_audio_feed")) return "audio_visualizer";
  if (source.includes("loi_play_samples") || source.includes("loi_samples_playing")) {
    return "sound_synth";
  }
  if (source.includes("loi_draw_text_slice") || source.includes("loi_measure_text_slice")) {
    return "text_editor";
  }
  if (/import\s+"[^"]*raylib\.loi"/.test(source)) return "raylib_web";
  return null;
}

function frameMessage(message) {
  const body = Buffer.from(JSON.stringify(message), "utf8");
  return Buffer.concat([
    Buffer.from(`Content-Length: ${body.length}\r\n\r\n`, "utf8"),
    body,
  ]);
}

function parseFrames(buffer) {
  const messages = [];
  let rest = buffer;
  while (true) {
    const split = rest.indexOf("\r\n\r\n");
    if (split < 0) break;
    const header = rest.subarray(0, split).toString("utf8");
    const match = /Content-Length:\s*(\d+)/i.exec(header);
    if (!match) throw new Error("The LSP server sent a frame without Content-Length.");
    const length = Number(match[1]);
    const start = split + 4;
    if (rest.length < start + length) break;
    messages.push(JSON.parse(rest.subarray(start, start + length).toString("utf8")));
    rest = rest.subarray(start + length);
  }
  return messages;
}

async function prepareLspSession() {
  const dir = await mkdtemp(path.join(os.tmpdir(), "loi-playground-lsp-"));
  try {
    const libTarget = path.join(dir, "lib");
    const examplesTarget = path.join(dir, "examples");
    await mkdir(libTarget, { recursive: true });
    await mkdir(examplesTarget, { recursive: true });
    for (const name of [...libraryModules, ...libraryShims]) {
      await copyFile(path.join(libDir, name), path.join(libTarget, name));
    }
    await copyFile(lspExamplePath, path.join(examplesTarget, "lsp_server.loi"));

    // Prefer the first-class `loi-lsp` executable produced by `cargo build`.
    // It speaks the same stdio protocol and rebuilds its embedded Loi server
    // only when the sources change. Keep the source build below as a fallback
    // for checkouts that only have the compiler.
    const configured = process.env.LOI_LSP_BIN
      ? path.resolve(process.cwd(), process.env.LOI_LSP_BIN)
      : null;
    const candidates = [
      configured,
      path.join(projectDir, "target", "debug", "loi-lsp"),
      path.join(projectDir, "target", "release", "loi-lsp"),
    ].filter((candidate) => candidate && existsSync(candidate));
    if (candidates.length > 0) {
      return { dir, binary: candidates[0] };
    }

    const binary = path.join(dir, "lsp-server");
    const build = await runProcess(
      compiler,
      [
        "build",
        "examples/lsp_server.loi",
        "-o",
        binary,
        ...libraryShims.flatMap((name) => ["--c-source", `lib/${name}`]),
      ],
      { cwd: dir, timeoutMs: BUILD_TIMEOUT_MS }
    );
    if (build.code !== 0 || isTimeout(build) || build.spawnError) {
      throw new Error(
        sanitize(build.stderr, dir) || `The LSP server exited with code ${build.code}.`
      );
    }
    return { dir, binary };
  } catch (error) {
    await rm(dir, { recursive: true, force: true }).catch(() => {});
    throw error;
  }
}

async function lspSession() {
  if (!lspSessionPromise) {
    lspSessionPromise = prepareLspSession().catch((error) => {
      lspSessionPromise = null;
      throw error;
    });
  }
  return lspSessionPromise;
}

/// Run a batch LSP session against the real server. The first-class `loi-lsp`
/// binary is reused when available; each request starts a short-lived process,
/// which keeps the playground stateless and avoids leaking editor sessions
/// between requests.
async function lspDiagnostics(source, changes = []) {
  const started = Date.now();
  const session = await lspSession();
  const uri = `file://${path.join(session.dir, "examples", "document.loi")}`;
  const messages = [
    { jsonrpc: "2.0", id: 1, method: "initialize", params: {} },
    { jsonrpc: "2.0", method: "initialized", params: {} },
    {
      jsonrpc: "2.0",
      method: "textDocument/didOpen",
      params: {
        textDocument: { uri, languageId: "loi", version: 1, text: source },
      },
    },
  ];
  changes.forEach((text, index) => {
    messages.push({
      jsonrpc: "2.0",
      method: "textDocument/didChange",
      params: {
        textDocument: { uri, version: index + 2 },
        contentChanges: [{ text }],
      },
    });
  });
  messages.push({ jsonrpc: "2.0", id: 99, method: "shutdown" });
  messages.push({ jsonrpc: "2.0", method: "exit" });

  const input = Buffer.concat(messages.map(frameMessage));
  const result = await new Promise((resolve) => {
    const child = spawn(session.binary, [], {
      cwd: session.dir,
      env: { ...process.env, LOI_COMPILER: compiler },
      stdio: ["pipe", "pipe", "pipe"],
    });
    const stdout = [];
    let stderr = "";
    let timedOut = false;
    let settled = false;
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill("SIGKILL");
    }, LSP_TIMEOUT_MS);
    child.stdout.on("data", (chunk) => stdout.push(chunk));
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => {
      if (stderr.length < MAX_OUTPUT_BYTES) stderr += chunk;
    });
    child.on("error", (error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve({ error: error.message });
    });
    child.on("close", () => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve({
        stdout: Buffer.concat(stdout),
        stderr,
        timedOut,
      });
    });
    child.stdin.on("error", () => {});
    child.stdin.end(input);
  });

  if (result.error || result.timedOut) {
    throw new Error(result.error || "The LSP server timed out.");
  }
  let messagesOut;
  try {
    messagesOut = parseFrames(result.stdout);
  } catch (error) {
    throw new Error(`${error.message} ${sanitize(result.stderr, session.dir)}`.trim());
  }
  return {
    ok: true,
    stage: "lsp",
    messages: messagesOut,
    stderr: sanitize(result.stderr, session.dir),
    durationMs: Date.now() - started,
  };
}

async function canBuildWeb() {
  if (webBuildSupport === null) {
    webBuildSupport = Promise.resolve().then(async () => {
      if (!process.env.RAYLIB_WEB) return false;
      const probe = await runProcess("emcc", ["--version"], { timeoutMs: 5_000 });
      return probe.code === 0 && !probe.spawnError;
    });
  }
  return webBuildSupport;
}

function webDemoSources(demo) {
  if (demo === "sound_synth" || demo === "audio_visualizer") {
    return ["examples/sound_shim.c"];
  }
  if (demo === "text_editor") return ["examples/editor_shim.c"];
  return [];
}

async function pruneWebRuns() {
  const now = Date.now();
  for (const name of readdirSync(webRunsDir, { withFileTypes: true })) {
    if (!name.isDirectory()) continue;
    const dir = path.join(webRunsDir, name.name);
    try {
      if (now - (await stat(dir)).mtimeMs > WEB_RUN_TTL_MS) {
        await rm(dir, { recursive: true, force: true });
      }
    } catch (_) {
      // A run may disappear while we inspect it; nothing to do.
    }
  }
}

/// Pick the execution target from the program itself. Console programs keep
/// using the native path; graphical/audio programs open a WebAssembly preview
/// because a browser cannot display a native window.
async function runWebDemo(source, demo) {
  const started = Date.now();
  const info = webDemos.get(demo);
  const prebuiltUrl = `/web/${demo}.html`;
  const fallbackReady = existsSync(path.join(websiteDir, "web", `${demo}.html`));
  const canBuild = await canBuildWeb();

  if (canBuild) {
    const token = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
    const runDir = path.join(webRunsDir, token);
    const workspace = await mkdtemp(path.join(os.tmpdir(), "loi-playground-web-"));
    try {
      const libTarget = path.join(workspace, "lib");
      const examplesTarget = path.join(workspace, "examples");
      await mkdir(libTarget, { recursive: true });
      await mkdir(examplesTarget, { recursive: true });
      await mkdir(path.join(workspace, "vendor"), { recursive: true });
      for (const name of libraryModules) {
        await copyFile(path.join(libDir, name), path.join(libTarget, name));
      }
      await cp(path.join(projectDir, "vendor", "raylib"), path.join(workspace, "vendor", "raylib"), {
        recursive: true,
      });
      const extraSources = webDemoSources(demo);
      for (const name of extraSources) {
        await copyFile(path.join(projectDir, name), path.join(examplesTarget, path.basename(name)));
      }
      await writeFile(path.join(examplesTarget, "main.loi"), source);
      await mkdir(runDir, { recursive: true });

      const build = await runProcess(
        "sh",
        [path.join(projectDir, "examples", "build_web.sh"), "examples/main.loi"],
        {
          cwd: workspace,
          timeoutMs: WEB_BUILD_TIMEOUT_MS,
          env: {
            ...process.env,
            LOI_BIN: compiler,
            OUT_DIR: runDir,
            RAYLIB_WEB: process.env.RAYLIB_WEB,
            WEB_C_SOURCES: extraSources
              .map((name) => `examples/${path.basename(name)}`)
              .join(" "),
          },
        }
      );
      if (build.code === 0 && existsSync(path.join(runDir, "main.html"))) {
        return {
          ok: true,
          stage: "web-build",
          mode: "web",
          demo,
          webUrl: `/web-runs/${token}/main.html`,
          caption: info?.caption || "",
          stdout: sanitize(build.stdout, workspace),
          stderr: sanitize(build.stderr, workspace),
          exitCode: 0,
          durationMs: Date.now() - started,
        };
      }
      if (!fallbackReady) {
        return {
          ok: false,
          stage: "web-build",
          mode: "web",
          demo,
          stdout: sanitize(build.stdout, workspace),
          stderr:
            sanitize(build.stderr, workspace) ||
            "The WebAssembly build failed and no prebuilt demo is available.",
          exitCode: build.code,
          durationMs: Date.now() - started,
        };
      }
      await rm(runDir, { recursive: true, force: true }).catch(() => {});
    } finally {
      await rm(workspace, { recursive: true, force: true }).catch(() => {});
    }
  }

  if (!fallbackReady) {
    return {
      ok: false,
      stage: "web-target",
      mode: "web",
      demo,
      stdout: "",
      stderr:
        `No WebAssembly build is available for ${demo}. Set RAYLIB_WEB and put emcc on PATH ` +
        "to compile this example in the playground.",
      exitCode: null,
      durationMs: Date.now() - started,
    };
  }
  return {
    ok: true,
    stage: "web-demo",
    mode: "web",
    demo,
    webUrl: prebuiltUrl,
    caption: info?.caption || "",
    stdout: "",
    stderr: "",
    exitCode: 0,
    durationMs: Date.now() - started,
  };
}

/* --------------------------------------------------------------------------
   Process and temporary workspace helpers
   -------------------------------------------------------------------------- */

function runProcess(command, args, options = {}) {
  const {
    cwd = projectDir,
    timeoutMs = 15_000,
    env = process.env,
  } = options;

  return new Promise((resolve) => {
    let stdout = "";
    let stderr = "";
    let timedOut = false;
    let settled = false;
    let child;
    let timer = null;

    const finish = (result) => {
      if (settled) return;
      settled = true;
      if (timer) clearTimeout(timer);
      resolve({
        stdout: stdout.slice(0, MAX_OUTPUT_BYTES),
        stderr: stderr.slice(0, MAX_OUTPUT_BYTES),
        timedOut,
        ...result,
      });
    };

    try {
      child = spawn(command, args, {
        cwd,
        env,
        stdio: ["ignore", "pipe", "pipe"],
      });
    } catch (error) {
      finish({ code: null, signal: null, spawnError: error });
      return;
    }

    timer = setTimeout(() => {
      timedOut = true;
      child.kill("SIGKILL");
    }, timeoutMs);

    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      if (stdout.length < MAX_OUTPUT_BYTES) stdout += chunk;
    });
    child.stderr.on("data", (chunk) => {
      if (stderr.length < MAX_OUTPUT_BYTES) stderr += chunk;
    });
    child.on("error", (error) => {
      finish({ code: null, signal: null, spawnError: error });
    });
    child.on("close", (code, signal) => {
      finish({ code, signal, spawnError: null });
    });
  });
}

async function prepareWorkspace(source) {
  const root = await mkdtemp(path.join(os.tmpdir(), "loi-playground-"));
  const examplesDir = path.join(root, "examples");
  const workspaceLibDir = path.join(root, "lib");
  const nestedLibDir = path.join(examplesDir, "lib");
  const raylibDir = path.join(root, "vendor", "raylib");

  await mkdir(examplesDir, { recursive: true });
  await mkdir(workspaceLibDir, { recursive: true });
  await mkdir(nestedLibDir, { recursive: true });
  await mkdir(raylibDir, { recursive: true });

  for (const name of [...libraryModules, ...libraryShims]) {
    const source = path.join(libDir, name);
    await copyFile(source, path.join(workspaceLibDir, name));
    await copyFile(source, path.join(nestedLibDir, name));
  }
  await copyFile(raylibPath, path.join(raylibDir, "raylib.loi"));

  const sourcePath = path.join(examplesDir, "main.loi");
  await writeFile(sourcePath, source);
  return { root, sourcePath };
}

async function withWorkspace(source, work) {
  let workspace;
  try {
    workspace = await prepareWorkspace(source);
  } catch (error) {
    return {
      ok: false,
      stage: "workspace",
      stdout: "",
      stderr: `Could not prepare the playground workspace: ${error.message}`,
      exitCode: null,
      durationMs: 0,
    };
  }

  try {
    return await work(workspace);
  } finally {
    await rm(workspace.root, { recursive: true, force: true }).catch(() => {});
  }
}

function sanitize(text, root) {
  if (!text) return "";
  const examples = `${root}${path.sep}examples${path.sep}`;
  return text
    .split(examples)
    .join("")
    .split(root)
    .join(".")
    .replace(/\r\n/g, "\n");
}

function isTimeout(result) {
  return result.timedOut || result.signal === "SIGKILL";
}

/* --------------------------------------------------------------------------
   Compiler operations
   -------------------------------------------------------------------------- */

async function compilerAvailable() {
  const result = await runProcess(compiler, ["--version"], { timeoutMs: 5_000 });
  if (isTimeout(result) || result.code !== 0 || result.spawnError) {
    return { ok: false, error: result.spawnError?.message || "compiler not found" };
  }
  return { ok: true, compiler: result.stdout.trim() };
}

async function checkSource(source) {
  const started = Date.now();
  return withWorkspace(source, async ({ root, sourcePath }) => {
    const result = await runProcess(
      compiler,
      ["check", sourcePath, "--error-format=short", "--color=never"],
      { cwd: root, timeoutMs: CHECK_TIMEOUT_MS }
    );
    const timedOut = isTimeout(result);
    return {
      ok: result.code === 0 && !timedOut,
      stage: timedOut ? "timeout" : "check",
      stdout: sanitize(result.stdout, root),
      stderr: timedOut
        ? "The checker timed out."
        : sanitize(result.stderr, root),
      exitCode: result.code,
      durationMs: Date.now() - started,
    };
  });
}

async function formatSource(source) {
  const started = Date.now();
  return withWorkspace(source, async ({ root, sourcePath }) => {
    const result = await runProcess(
      compiler,
      ["fmt", sourcePath],
      { cwd: root, timeoutMs: FORMAT_TIMEOUT_MS }
    );
    const timedOut = isTimeout(result);
    const ok = result.code === 0 && !timedOut;
    return {
      ok,
      stage: timedOut ? "timeout" : "format",
      formatted: ok ? result.stdout : undefined,
      stdout: "",
      stderr: timedOut
        ? "The formatter timed out."
        : sanitize(result.stderr, root),
      exitCode: result.code,
      durationMs: Date.now() - started,
    };
  });
}

async function runSource(source) {
  const demo = detectWebDemo(source);
  if (demo) return runWebDemo(source, demo);

  const started = Date.now();
  return withWorkspace(source, async ({ root, sourcePath }) => {
    const outputPath = path.join(root, "main");
    const build = await runProcess(
      compiler,
      [
        "build",
        sourcePath,
        "-o",
        outputPath,
        "--error-format=short",
        "--color=never",
        ...playgroundShims.flatMap((name) => ["--c-source", `lib/${name}`]),
      ],
      { cwd: root, timeoutMs: BUILD_TIMEOUT_MS }
    );

    if (isTimeout(build)) {
      return {
        ok: false,
        stage: "compile",
        stdout: sanitize(build.stdout, root),
        stderr: "Compilation timed out.",
        exitCode: null,
        durationMs: Date.now() - started,
      };
    }

    if (build.code !== 0) {
      const stderr = sanitize(build.stderr, root);
      return {
        ok: false,
        stage: "compile",
        stdout: sanitize(build.stdout, root),
        stderr: stderr || `The compiler exited with code ${build.code}.`,
        exitCode: build.code,
        durationMs: Date.now() - started,
      };
    }

    const run = await runProcess(outputPath, [], {
      cwd: root,
      timeoutMs: RUN_TIMEOUT_MS,
    });
    const timedOut = isTimeout(run);
    const stdout = sanitize(run.stdout, root);
    let stderr = sanitize(run.stderr, root);
    if (run.spawnError) {
      stderr = `Could not start the compiled program: ${run.spawnError.message}`;
    } else if (timedOut) {
      stderr = [stderr, "Execution timed out after 10 seconds."].filter(Boolean).join("\n");
    } else if (run.code !== 0 && !stderr) {
      stderr = `The program exited with code ${run.code}.`;
    }

    return {
      ok: run.code === 0 && !timedOut && !run.spawnError,
      stage: timedOut ? "run-timeout" : "run",
      stdout,
      stderr,
      exitCode: timedOut ? null : run.code,
      durationMs: Date.now() - started,
      timedOut,
    };
  });
}

/* --------------------------------------------------------------------------
   HTTP helpers
   -------------------------------------------------------------------------- */

const MIME_TYPES = new Map([
  [".html", "text/html; charset=utf-8"],
  [".css", "text/css; charset=utf-8"],
  [".js", "text/javascript; charset=utf-8"],
  [".mjs", "text/javascript; charset=utf-8"],
  [".json", "application/json; charset=utf-8"],
  [".svg", "image/svg+xml"],
  [".png", "image/png"],
  [".ico", "image/x-icon"],
  [".loi", "text/plain; charset=utf-8"],
  [".md", "text/markdown; charset=utf-8"],
  [".txt", "text/plain; charset=utf-8"],
  [".wasm", "application/wasm"],
]);

function securityHeaders(res) {
  res.setHeader("X-Content-Type-Options", "nosniff");
  res.setHeader("Referrer-Policy", "no-referrer");
  res.setHeader("X-Frame-Options", "SAMEORIGIN");
  res.setHeader(
    "Content-Security-Policy",
    "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; " +
      "script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'; base-uri 'none'; " +
      "form-action 'none'; frame-ancestors 'self'"
  );
}

function sendJson(res, status, payload) {
  const body = JSON.stringify(payload);
  securityHeaders(res);
  res.writeHead(status, {
    "Content-Type": "application/json; charset=utf-8",
    "Cache-Control": "no-store",
    "Content-Length": Buffer.byteLength(body),
  });
  res.end(body);
}

function sendText(res, status, text, contentType = "text/plain; charset=utf-8") {
  securityHeaders(res);
  res.writeHead(status, {
    "Content-Type": contentType,
    "Cache-Control": "no-store",
    "Content-Length": Buffer.byteLength(text),
  });
  res.end(text);
}

async function readJsonBody(req) {
  const chunks = [];
  let size = 0;

  for await (const chunk of req) {
    size += chunk.length;
    if (size > MAX_BODY_BYTES) {
      const error = new Error("Request body is too large.");
      error.status = 413;
      throw error;
    }
    chunks.push(chunk);
  }

  if (size === 0) {
    const error = new Error("Request body is empty.");
    error.status = 400;
    throw error;
  }

  let payload;
  try {
    payload = JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch (_) {
    const error = new Error("Request body must be valid JSON.");
    error.status = 400;
    throw error;
  }

  if (!payload || typeof payload.source !== "string") {
    const error = new Error("Expected a JSON object with a string `source` field.");
    error.status = 400;
    throw error;
  }

  if (!payload.source.trim()) {
    const error = new Error("The program is empty.");
    error.status = 400;
    throw error;
  }

  if (Buffer.byteLength(payload.source, "utf8") > MAX_SOURCE_BYTES) {
    const error = new Error("Programs are limited to 200 KB in the playground.");
    error.status = 413;
    throw error;
  }

  if (
    payload.changes !== undefined &&
    (!Array.isArray(payload.changes) ||
      payload.changes.some((item) => typeof item !== "string" ||
        Buffer.byteLength(item, "utf8") > MAX_SOURCE_BYTES))
  ) {
    const error = new Error("`changes` must be an array of Loi source strings.");
    error.status = 400;
    throw error;
  }
  if (payload.demo !== undefined && typeof payload.demo !== "string") {
    const error = new Error("`demo` must be a string.");
    error.status = 400;
    throw error;
  }

  return payload;
}

async function serveStatic(req, res, url) {
  let pathname;
  try {
    pathname = decodeURIComponent(url.pathname);
  } catch (_) {
    sendText(res, 400, "Bad request: malformed URL.");
    return;
  }

  if (pathname.includes("\0")) {
    sendText(res, 400, "Bad request.");
    return;
  }

  const relative = pathname === "/" ? "index.html" : pathname.replace(/^\/+/, "");
  let filePath = path.resolve(websiteDir, relative);
  const prefix = websiteDir + path.sep;
  if (filePath !== websiteDir && !filePath.startsWith(prefix)) {
    sendText(res, 403, "Forbidden.");
    return;
  }

  let info;
  try {
    info = await stat(filePath);
  } catch (_) {
    const fallback = path.join(websiteDir, "404.html");
    try {
      const body = await readFile(fallback);
      securityHeaders(res);
      res.writeHead(404, {
        "Content-Type": "text/html; charset=utf-8",
        "Content-Length": body.length,
        "Cache-Control": "no-cache",
      });
      res.end(req.method === "HEAD" ? undefined : body);
    } catch (_) {
      sendText(res, 404, "Not found.");
    }
    return;
  }

  if (info.isDirectory()) {
    filePath = path.join(filePath, "index.html");
    try {
      info = await stat(filePath);
    } catch (_) {
      sendText(res, 404, "Not found.");
      return;
    }
  }

  if (!info.isFile()) {
    sendText(res, 404, "Not found.");
    return;
  }

  let body;
  try {
    body = await readFile(filePath);
  } catch (error) {
    sendText(res, 500, `Could not read ${path.basename(filePath)}: ${error.message}`);
    return;
  }

  const extension = path.extname(filePath).toLowerCase();
  const contentType = MIME_TYPES.get(extension) || "application/octet-stream";
  securityHeaders(res);
  res.writeHead(200, {
    "Content-Type": contentType,
    "Content-Length": body.length,
    "Cache-Control": extension === ".html" ? "no-cache" : "public, max-age=300",
  });
  res.end(req.method === "HEAD" ? undefined : body);
}

/* --------------------------------------------------------------------------
   API routing
   -------------------------------------------------------------------------- */

async function handleApi(req, res, url) {
  const route = url.pathname;

  if (route === "/api/health") {
    if (req.method !== "GET" && req.method !== "HEAD") {
      sendJson(res, 405, { ok: false, error: "Use GET." });
      return;
    }
    const health = await compilerAvailable();
    let lib = true;
    try {
      await stat(libPath);
    } catch (_) {
      lib = false;
    }
    sendJson(res, 200, {
      ok: health.ok && lib,
      compiler: health.compiler || null,
      error: health.error || (lib ? undefined : "lib/std.loi is missing"),
    });
    return;
  }

  if (!["/api/check", "/api/format", "/api/run", "/api/run-web", "/api/lsp"].includes(route)) {
    sendJson(res, 404, { ok: false, error: `Unknown API endpoint: ${route}` });
    return;
  }

  if (req.method !== "POST") {
    sendJson(res, 405, { ok: false, error: "Use POST with a JSON body." });
    return;
  }

  if (activeJobs >= MAX_CONCURRENT) {
    sendJson(res, 503, {
      ok: false,
      error: "The playground is busy. Try again in a moment.",
    });
    return;
  }

  let payload;
  try {
    payload = await readJsonBody(req);
  } catch (error) {
    sendJson(res, error.status || 400, { ok: false, error: error.message });
    return;
  }

  activeJobs += 1;
  const started = Date.now();
  try {
    let result;
    if (route === "/api/check") {
      result = await checkSource(payload.source);
    } else if (route === "/api/format") {
      result = await formatSource(payload.source);
    } else if (route === "/api/lsp") {
      result = await lspDiagnostics(payload.source, payload.changes || []);
    } else if (route === "/api/run-web") {
      const demo = detectWebDemo(payload.source) || payload.demo;
      if (!demo || !webDemos.has(demo)) {
        result = {
          ok: false,
          stage: "web-target",
          mode: "web",
          stdout: "",
          stderr: "Unknown WebAssembly demo.",
          exitCode: null,
          durationMs: 0,
        };
      } else {
        result = await runWebDemo(payload.source, demo);
      }
    } else {
      result = await runSource(payload.source);
    }
    console.log(
      `[loi] ${route} ${result.ok ? "ok" : "failed"} ` +
        `${result.stage || ""} ${result.durationMs || Date.now() - started}ms`
    );
    sendJson(res, 200, result);
  } catch (error) {
    console.error(`[loi] ${route} crashed:`, error);
    sendJson(res, 500, {
      ok: false,
      stage: "server",
      stdout: "",
      stderr: `The playground server failed: ${error.message}`,
      exitCode: null,
      durationMs: Date.now() - started,
    });
  } finally {
    activeJobs -= 1;
  }
}

const server = http.createServer((req, res) => {
  let url;
  try {
    url = new URL(req.url || "/", "http://localhost");
  } catch (_) {
    sendText(res, 400, "Bad request: malformed URL.");
    return;
  }

  if (url.pathname === "/api" || url.pathname.startsWith("/api/")) {
    handleApi(req, res, url).catch((error) => {
      console.error("[loi] unhandled API error:", error);
      if (!res.headersSent) {
        sendJson(res, 500, { ok: false, error: "Internal server error." });
      } else {
        res.destroy();
      }
    });
    return;
  }

  if (req.method !== "GET" && req.method !== "HEAD") {
    sendText(res, 405, "Method not allowed.");
    return;
  }

  serveStatic(req, res, url).catch((error) => {
    console.error("[loi] static error:", error);
    if (!res.headersSent) {
      sendText(res, 500, "Internal server error.");
    } else {
      res.destroy();
    }
  });
});

server.requestTimeout = 120_000;
server.headersTimeout = 20_000;

async function main() {
  try {
    await stat(libPath);
  } catch (_) {
    console.error(`[loi] warning: ${libPath} is missing; library imports will fail.`);
  }

  const health = await compilerAvailable();
  if (!health.ok) {
    console.error(
      `[loi] warning: could not run '${compiler} --version' (${health.error}). ` +
        "Build the compiler with `cargo build` or set LOI_BIN."
    );
  } else {
    console.log(`[loi] compiler: ${health.compiler}`);
  }

  if (existsSync(webRunsDir)) await pruneWebRuns();

  server.listen(PORT, HOST, () => {
    console.log(`[loi] playground listening on http://${HOST}:${PORT}/playground.html`);
    console.log(`[loi] serving ${websiteDir}`);
  });
}

main().catch((error) => {
  console.error("[loi] failed to start:", error);
  process.exitCode = 1;
});
