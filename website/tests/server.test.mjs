// End-to-end tests for the playground server. They drive the real compiler and
// the real LSP server; no browser or npm dependencies are required.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";
import path from "node:path";

const websiteDir = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const projectDir = path.dirname(websiteDir);
let child;
let base;
const webServers = [];

async function spawnWebServer(extraEnv = {}) {
  const port = await freePort();
  const proc = spawn(process.execPath, ["website/server.mjs"], {
    cwd: projectDir,
    env: {
      ...process.env,
      HOST: "127.0.0.1",
      PORT: String(port),
      LOI_BIN: "target/debug/loi",
      LOI_LSP_BIN: "target/debug/loi-lsp",
      ...extraEnv,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  proc.stderr.on("data", (chunk) => process.stderr.write(`[web server] ${chunk}`));
  webServers.push(proc);
  const serverBase = `http://127.0.0.1:${port}`;
  await waitForServer(serverBase);
  return { proc, base: serverBase };
}


function freePort() {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
  });
}

async function waitForServer(serverBase = base) {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${serverBase}/api/health`);
      if (response.ok) return;
    } catch (_) {
      // Server is still starting.
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error("Playground server did not start in time.");
}

async function post(route, payload, serverBase = base) {
  const response = await fetch(`${serverBase}${route}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
  assert.equal(response.status, 200, `${route} returned HTTP ${response.status}`);
  return response.json();
}

before(async () => {
  const port = await freePort();
  base = `http://127.0.0.1:${port}`;
  child = spawn(process.execPath, ["website/server.mjs"], {
    cwd: projectDir,
    env: {
      ...process.env,
      HOST: "127.0.0.1",
      PORT: String(port),
      LOI_BIN: "target/debug/loi",
      // Exercise the first-class binary; server.mjs still falls back to a
      // source build when it is absent.
      LOI_LSP_BIN: "target/debug/loi-lsp",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  child.stderr.on("data", (chunk) => process.stderr.write(chunk));
  await waitForServer();
});

after(() => {
  for (const proc of [...webServers, child]) {
    if (proc && !proc.killed) proc.kill("SIGTERM");
  }
});

test("health reports the real compiler", async () => {
  const result = await (await fetch(`${base}/api/health`)).json();
  assert.equal(result.ok, true);
  assert.match(result.compiler, /loi/);
});

test("run compiles and executes an ordinary program", async () => {
  const result = await post("/api/run", {
    source: 'open flow main\n    => i32\n{\n    print("hello from playground\\n")\n    => 0\n}\n',
  });
  assert.equal(result.ok, true, result.stderr);
  assert.equal(result.mode, undefined);
  assert.match(result.stdout, /hello from playground/);
});

test("run resolves every library module in the submitted program", async () => {
  const modules = ["std", "json", "jsonrpc", "lsp", "process", "os", "vec", "math", "ctype"];
  const imports = modules.map((name) => `import "../lib/${name}.loi"`).join("\n");
  const result = await post("/api/check", {
    source: `${imports}\n\nopen flow main\n    => i32\n{\n    => 0\n}\n`,
  });
  assert.equal(result.ok, true, result.stderr);
});

test("raylib programs are routed to the WebAssembly preview", async () => {
  const response = await fetch(`${base}/examples/raylib_web.loi`);
  assert.equal(response.ok, true);
  const source = await response.text();
  const result = await post("/api/run", { source });
  assert.equal(result.ok, true, result.stderr);
  assert.equal(result.mode, "web");
  assert.equal(result.demo, "raylib_web");

  const preview = await fetch(`${base}${result.webUrl}`);
  assert.equal(preview.status, 200);
  assert.match(await preview.text(), /<canvas/);
});

test("browser-submitted programs cannot link the process shim", async () => {
  const result = await post("/api/run", {
    source:
      'import "../lib/process.loi"\n\n' +
      "open flow main\n    => i32\n{\n" +
      '    guard child <- process_spawn("printf should-not-run") {\n' +
      "    }\n    => 0\n}\n",
  });
  assert.equal(result.ok, false);
  assert.doesNotMatch(result.stderr, /should-not-run/);
});

test("playground has one WebAssembly preview and no duplicated gallery", async () => {
  const response = await fetch(`${base}/playground.html`);
  const html = await response.text();
  assert.doesNotMatch(html, /web-demo|Emscripten|galeria/i);
  assert.match(html, /id="preview-frame"[^>]*allow="autoplay"/);
});

test("audio visualizer keeps its animation alive before the audio unlock", async () => {
  const source = await readFile(path.join(projectDir, "examples", "audio_visualizer.loi"), "utf8");
  const generate = source.indexOf("generate_audio(<-> samples)");
  const ready = source.indexOf("if loi_audio_ready()");
  assert.ok(
    generate >= 0 && ready >= 0 && generate < ready,
    "generate_audio must run while the AudioContext is still suspended"
  );
  // A `->` output parameter resets the slice to zero length, so the first
  // `out[i]` aborts the program before it draws a single frame. The caller
  // allocates the buffer once; generate_audio must borrow it mutably.
  assert.match(source, /flow generate_audio\s*\n\s*<-> out: \[f32\]/);
  assert.match(source, /generate_audio\(<-> samples\)/);
  const mirror = await readFile(
    path.join(projectDir, "website", "examples", "audio_visualizer.loi"),
    "utf8"
  );
  assert.equal(mirror, source);
});

test("audio examples use the shim instead of raylib's WebAudio device", async () => {
  const shim = await readFile(path.join(projectDir, "examples", "sound_shim.c"), "utf8");
  assert.match(shim, /void loi_init_audio_device\(void\)/);
  assert.match(shim, /void loi_close_audio_device\(void\)/);
  for (const name of ["audio_visualizer", "sound_synth"]) {
    const source = await readFile(path.join(projectDir, "examples", `${name}.loi`), "utf8");
    assert.match(source, /extern flow loi_init_audio_device/);
    assert.match(source, /extern flow loi_close_audio_device/);
    assert.match(source, /loi_init_audio_device\(\)/);
    assert.match(source, /loi_close_audio_device\(\)/);
    assert.doesNotMatch(source, /^\s*(InitAudioDevice|CloseAudioDevice)\(\)/m);
  }
});

test("audio visualizer preview carries the unlock hint", async () => {
  const response = await fetch(`${base}/web/audio_visualizer.html`);
  assert.equal(response.status, 200);
  const html = await response.text();
  assert.match(html, /audio_visualizer\.loader\.js/);
  assert.match(html, /liberar o audio/);
});

test("LSP endpoint publishes diagnostics from the real compiler", async () => {
  const result = await post("/api/lsp", {
    source: "open flow main\n    => i32\n{\n    => missing\n}\n",
  });
  assert.equal(result.ok, true, result.stderr);
  const notifications = result.messages.filter(
    (message) => message.method === "textDocument/publishDiagnostics"
  );
  assert.equal(notifications.length, 1);
  assert.equal(notifications[0].params.diagnostics.length, 1);
  assert.match(notifications[0].params.diagnostics[0].message, /unknown variable/);
});

test("identical WebAssembly builds are cached and reused", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "loi-web-cache-test-"));
  const bin = path.join(root, "bin");
  const raylib = path.join(root, "raylib-web");
  const counter = path.join(root, "builds");
  const cache = path.join(root, "web-runs");
  await mkdir(bin, { recursive: true });
  await mkdir(raylib, { recursive: true });
  await mkdir(cache, { recursive: true });
  await writeFile(counter, "0");
  await writeFile(
    path.join(bin, "emcc"),
    `#!/usr/bin/env sh\nif [ "$1" = "--version" ]; then echo emcc test; exit 0; fi\nexit 0\n`
  );
  await writeFile(
    path.join(bin, "build_web.sh"),
    `#!/usr/bin/env sh
set -eu
count=$(cat "$BUILD_COUNTER")
echo $((count + 1)) > "$BUILD_COUNTER"
mkdir -p "$OUT_DIR"
cat > "$OUT_DIR/main.html" <<EOF
<html><body><canvas></canvas></body></html>
EOF
`
  );
  await chmod(path.join(bin, "emcc"), 0o755);
  await chmod(path.join(bin, "build_web.sh"), 0o755);

  try {
    const server = await spawnWebServer({
      PATH: `${bin}:${process.env.PATH}`,
      RAYLIB_WEB: raylib,
      LOI_WEB_RUNS_DIR: cache,
      LOI_WEB_BUILD_SCRIPT: path.join(bin, "build_web.sh"),
      BUILD_COUNTER: counter,
    });
    const source = await readFile(path.join(projectDir, "examples", "raylib_web.loi"), "utf8");
    const first = await post("/api/run", { source }, server.base);
    assert.equal(first.ok, true, first.stderr);
    assert.equal(first.stage, "web-build");
    assert.equal((await readFile(counter, "utf8")).trim(), "1");

    const second = await post("/api/run", { source }, server.base);
    assert.equal(second.ok, true, second.stderr);
    assert.equal(second.stage, "web-cache");
    assert.equal(second.webUrl, first.webUrl);
    assert.equal((await readFile(counter, "utf8")).trim(), "1");

    const edited = source.replace("open flow main", "open flow changed");
    const third = await post("/api/run", { source: edited }, server.base);
    assert.equal(third.ok, true, third.stderr);
    assert.equal(third.stage, "web-build");
    assert.notEqual(third.webUrl, first.webUrl);
    assert.equal((await readFile(counter, "utf8")).trim(), "2");
  } finally {
    for (const proc of webServers.splice(0)) {
      if (!proc.killed) proc.kill("SIGTERM");
    }
    await rm(root, { recursive: true, force: true });
  }
});
