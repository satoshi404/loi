/* Loi playground: editor, syntax overlay, compiler API and LSP client. */
(function () {
  "use strict";

  const editor = document.getElementById("editor");
  const highlight = document.getElementById("highlight");
  const gutter = document.getElementById("gutter");
  const output = document.getElementById("output");
  const meta = document.getElementById("output-meta");
  const status = document.getElementById("playground-status");
  const lspStatus = document.getElementById("lsp-status");
  const preset = document.getElementById("preset");
  const problemsList = document.getElementById("problems-list");
  const problemCount = document.getElementById("problem-count");
  const previewFrame = document.getElementById("preview-frame");
  const previewCaption = document.getElementById("preview-caption");
  const previewOpen = document.getElementById("preview-open");
  const runButton = document.querySelector('[data-action="run"]');
  const checkButton = document.querySelector('[data-action="check"]');
  const formatButton = document.querySelector('[data-action="format"]');
  const lspButton = document.querySelector('[data-action="lsp"]');
  if (!editor || !highlight || !gutter || !output) return;

  const STORAGE_KEY = "loi-playground-source";
  const MAX_LENGTH = 200_000;
  const LSP_DEBOUNCE_MS = 400;

  const PRESETS = {
    hello: {
      label: "Hello, Loi",
      code: `shape Greeting {
    message: [u8]
    times: u32
}

open flow greet
    <- greeting: Greeting
{
    for i in 0..greeting.times {
        print(greeting.message)
    }
}

open flow main
    => i32
{
    greet(Greeting { message: "Hello, Loi!\\n", times: 3 })
    print("The playground runs the real compiler.\\n")
    => 0
}
`,
    },
    directions: {
      label: "Directions & flows",
      code: `shape Counter {
    value: i32
}

flow bump
    <-> counter: Counter
    <- amount: i32
{
    counter.value += amount
}

flow classify
    <- counter: Counter
    => [u8]
{
    if counter.value > 0 {
        => "positive"
    }
    if counter.value < 0 {
        => "negative"
    }
    => "zero"
}

open flow main
    => i32
{
    mut counter: Counter <- Counter { value: 0 }
    bump(<-> counter, 3)
    print("counter is ")
    print(classify(counter))
    print("\\n")
    => 0
}
`,
    },
    ownership: {
      label: "Ownership & regions",
      code: `shape Point {
    x: i32
    y: i32
}

open flow make_point
    -> point: Point
{
    point = Point { x: 10, y: 20 }
}

open flow main
    => i32
{
    mut point: Point
    make_point(-> point)
    // clone is explicit: composite values never copy implicitly.
    copy <- clone(point)
    protect {
        mut xs: [i32]
        alloc(-> xs, i32, 4)
        xs[0] = copy.x
        xs[1] = copy.y
        xs[2] = xs[0] + xs[1]
        xs[3] = 0
        print("allocated four integers inside a protect region\\n")
    }
    => 0
}
`,
    },
    tictactoe: {
      label: "Tic-tac-toe",
      url: "examples/tictactoe.loi",
    },
    lsp_server: {
      label: "LSP server (Loi)",
      url: "examples/lsp_server.loi",
      note: "Este é o servidor LSP que alimenta o painel Problems. Use Check/Format; o painel LSP executa o servidor real com os shims de processo, que não são linkados em programas enviados pelo navegador.",
    },
    raylib_web: {
      label: "Raylib web demo",
      url: "examples/raylib_web.loi",
      note: "Raylib detectado: Run abre a janela WebAssembly na aba Preview.",
    },
    text_editor: {
      label: "Simple text editor",
      url: "examples/text_editor.loi",
      note:
        "Editor com raylib: Run usa um build WebAssembly quando emcc/RAYLIB_WEB estão disponíveis.",
    },
    sound_synth: {
      label: "Math sound + waveform",
      url: "examples/sound_synth.loi",
      note: "Síntese de som: Run abre o preview com WebAudio.",
    },
    audio_visualizer: {
      label: "Audio visualizer",
      url: "examples/audio_visualizer.loi",
      note: "Visualizador: Run abre o preview com áudio e renderização.",
    },
  };

  let busy = false;
  let lspEnabled = true;
  let lspBusy = false;
  let lspDirty = false;
  let lspTimer = null;
  let diagnostics = [];

  function renderEditor() {
    const source = editor.value;
    highlight.innerHTML = window.LoiHighlight
      ? window.LoiHighlight.highlight(source + "\n")
      : "";
    const lines = source.split("\n").length;
    gutter.textContent = Array.from({ length: lines }, (_, index) => index + 1).join("\n");
    syncScroll();
  }

  function syncScroll() {
    highlight.scrollTop = editor.scrollTop;
    highlight.scrollLeft = editor.scrollLeft;
    gutter.scrollTop = editor.scrollTop;
  }

  function setStatus(text, state, element = status) {
    if (!element) return;
    element.classList.remove("is-online", "is-offline");
    if (state) element.classList.add(state);
    const label = element.querySelector("span");
    if (label) label.textContent = text;
  }

  function setBusy(value) {
    busy = value;
    [runButton, checkButton, formatButton].forEach((button) => {
      if (button) button.disabled = value;
    });
  }

  function formatDuration(ms) {
    if (typeof ms !== "number") return "";
    if (ms < 1000) return `${Math.round(ms)} ms`;
    return `${(ms / 1000).toFixed(2)} s`;
  }

  function showOutput(text, state) {
    output.innerHTML = "";
    const pre = document.createElement("span");
    pre.textContent = text;
    output.appendChild(pre);
  }

  function updateMeta(result, fallbackStage) {
    if (!meta) return;
    const stage = result.stage || fallbackStage;
    const ok = result.ok;
    const exit = typeof result.exitCode === "number" ? `exit ${result.exitCode}` : "no exit code";
    meta.innerHTML = "";
    const badge = document.createElement("span");
    badge.className = ok ? "ok" : "err";
    badge.textContent = ok ? "success" : "failed";
    meta.appendChild(badge);
    const details = document.createElement("span");
    details.textContent = [result.mode === "web" ? "webassembly" : stage, exit, formatDuration(result.durationMs)]
      .filter(Boolean)
      .join(" · ");
    meta.appendChild(details);
  }

  function setOutputTab(name) {
    document.querySelectorAll("[data-output-tab]").forEach((tab) => {
      const active = tab.getAttribute("data-output-tab") === name;
      tab.classList.toggle("is-active", active);
      tab.setAttribute("aria-selected", active ? "true" : "false");
    });
    document.querySelectorAll("[data-output-view]").forEach((view) => {
      view.hidden = view.getAttribute("data-output-view") !== name;
    });
  }

  function offsetAt(line, character) {
    const lines = editor.value.split("\n");
    let offset = 0;
    for (let index = 0; index < Math.min(line, lines.length); index += 1) {
      offset += lines[index].length + 1;
    }
    return offset + character;
  }

  function revealLocation(line) {
    const offset = offsetAt(line, 0);
    editor.focus();
    editor.setSelectionRange(offset, offset);
    const lineHeight = 22;
    editor.scrollTop = Math.max(0, line * lineHeight - editor.clientHeight / 2);
    syncScroll();
  }

  function renderProblems(items) {
    diagnostics = items;
    if (!problemsList) return;
    problemsList.innerHTML = "";
    const count = items.length;
    if (problemCount) {
      problemCount.hidden = count === 0;
      problemCount.textContent = String(count);
    }
    if (count === 0) {
      const placeholder = document.createElement("p");
      placeholder.className = "placeholder";
      placeholder.textContent = "Nenhum diagnóstico. O servidor Loi está limpo.";
      problemsList.appendChild(placeholder);
      return;
    }
    items.forEach((item) => {
      const range = item.range || { start: { line: 0, character: 0 } };
      const button = document.createElement("button");
      button.type = "button";
      button.className = "problem";
      if (item.severity === 2) button.classList.add("is-warning");
      if (item.severity === 3) button.classList.add("is-note");
      const dot = document.createElement("span");
      dot.className = "problem-severity";
      dot.setAttribute("aria-hidden", "true");
      const body = document.createElement("span");
      const where = document.createElement("span");
      where.className = "problem-location";
      where.textContent = `${range.start.line + 1}:${range.start.character + 1}`;
      const message = document.createElement("span");
      message.textContent = ` ${item.message}`;
      body.append(where, message);
      button.append(dot, body);
      button.addEventListener("click", () => revealLocation(range.start.line));
      problemsList.appendChild(button);
    });
  }

  function showWebPreview(result) {
    const url = result.webUrl;
    if (!url || !previewFrame) return;
    previewFrame.src = url;
    previewFrame.hidden = false;
    if (previewCaption) {
      previewCaption.textContent =
        result.caption || "Programa WebAssembly em execução. Clique no canvas para dar foco/áudio.";
    }
    if (previewOpen) {
      previewOpen.href = url;
      previewOpen.hidden = false;
    }
    setOutputTab("preview");
  }

  async function api(path, payload) {
    const response = await fetch(path, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });

    let result;
    try {
      result = await response.json();
    } catch (_) {
      throw new Error(`The playground server returned HTTP ${response.status} without JSON.`);
    }

    if (!response.ok && result.error) {
      throw new Error(result.error);
    }
    return result;
  }

  async function run(mode) {
    if (busy) return;
    const source = editor.value;
    if (!source.trim()) {
      showOutput("Write some Loi code first.", "error");
      return;
    }
    if (source.length > MAX_LENGTH) {
      showOutput("Programs are limited to 200 KB in the playground.", "error");
      return;
    }

    setBusy(true);
    showOutput(mode === "format" ? "Formatando…" : "Compilando…");
    if (meta) meta.textContent = "";

    try {
      const result = await api(`/api/${mode}`, { source });

      if (mode === "format" && result.ok) {
        editor.value = result.formatted;
        localStorage.setItem(STORAGE_KEY, result.formatted);
        renderEditor();
        showOutput("Formatado com o formatador canônico Flow Syntax.\n");
        updateMeta(result, mode);
        scheduleLsp();
        return;
      }

      if (result.mode === "web" && result.webUrl) {
        showWebPreview(result);
        showOutput(
          result.stage === "web-demo"
            ? "Abrindo o build WebAssembly pré-compilado na aba Preview."
            : "Programa compilado para WebAssembly. Use a aba Preview.",
          result.ok ? "ok" : "error"
        );
        if (!result.ok && result.stderr) showOutput(result.stderr, "error");
        updateMeta(result, mode);
        return;
      }

      const parts = [];
      if (result.stdout) parts.push(result.stdout.trimEnd());
      if (result.stderr) parts.push(result.stderr.trimEnd());
      if (!parts.length) {
        parts.push(
          result.ok
            ? mode === "check"
              ? "Check passed — no diagnostics."
              : "Program finished without output."
            : "The compiler reported an error without text."
        );
      }
      showOutput(parts.join("\n\n"), result.ok ? "ok" : "error");
      updateMeta(result, mode);
      setOutputTab("console");
    } catch (error) {
      const hint =
        "\n\nIs the playground server running?\n\n" +
        "  cargo build\n" +
        "  node website/server.mjs\n";
      showOutput(`${error.message}${hint}`, "error");
      if (meta) {
        meta.innerHTML = "";
        const badge = document.createElement("span");
        badge.className = "err";
        badge.textContent = "offline";
        meta.appendChild(badge);
      }
      setStatus("Playground server offline", "is-offline");
    } finally {
      setBusy(false);
    }
  }

  function scheduleLsp() {
    if (!lspEnabled) return;
    if (lspTimer) clearTimeout(lspTimer);
    lspTimer = setTimeout(() => {
      lspTimer = null;
      requestLsp();
    }, LSP_DEBOUNCE_MS);
  }

  async function requestLsp() {
    if (!lspEnabled || lspBusy) {
      lspDirty = true;
      return;
    }
    lspBusy = true;
    try {
      const result = await api("/api/lsp", { source: editor.value });
      const items = [];
      (result.messages || []).forEach((message) => {
        if (message.method !== "textDocument/publishDiagnostics") return;
        (message.params?.diagnostics || []).forEach((item) => {
          items.push({ ...item, uri: message.params.uri });
        });
      });
      renderProblems(items);
      setStatus("LSP online", "is-online", lspStatus);
    } catch (_) {
      setStatus("LSP offline", "is-offline", lspStatus);
    } finally {
      lspBusy = false;
      if (lspDirty) {
        lspDirty = false;
        scheduleLsp();
      }
    }
  }

  async function loadPreset(name) {
    const item = PRESETS[name];
    if (!item) return;
    if (item.url) {
      try {
        const response = await fetch(item.url);
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        editor.value = await response.text();
      } catch (_) {
        showOutput(
          `Could not load ${item.label}. Start the playground server to load it from the website examples directory.`,
          "error"
        );
        return;
      }
    } else {
      editor.value = item.code;
    }
    localStorage.setItem(STORAGE_KEY, editor.value);
    renderEditor();
    showOutput(item.note || "Loaded preset. Press Run to compile and execute it.");
    if (meta) meta.textContent = "";
    scheduleLsp();
  }

  editor.addEventListener("input", () => {
    localStorage.setItem(STORAGE_KEY, editor.value);
    renderEditor();
    scheduleLsp();
  });
  editor.addEventListener("scroll", syncScroll, { passive: true });
  editor.addEventListener("keydown", (event) => {
    if (event.key === "Tab") {
      event.preventDefault();
      const start = editor.selectionStart;
      const end = editor.selectionEnd;
      editor.setRangeText("    ", start, end, "end");
      localStorage.setItem(STORAGE_KEY, editor.value);
      renderEditor();
    } else if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
      event.preventDefault();
      run(event.shiftKey ? "check" : "run");
    }
  });

  const clearButton = document.getElementById("clear-output");
  if (clearButton) {
    clearButton.addEventListener("click", () => {
      output.innerHTML = "";
      const placeholder = document.createElement("span");
      placeholder.className = "placeholder";
      placeholder.textContent = "Output cleared.";
      output.appendChild(placeholder);
      if (meta) meta.textContent = "";
    });
  }

  document.querySelectorAll("[data-output-tab]").forEach((tab) => {
    tab.addEventListener("click", () => setOutputTab(tab.getAttribute("data-output-tab")));
  });

  if (lspButton) {
    lspButton.classList.toggle("btn-primary", lspEnabled);
    lspButton.addEventListener("click", () => {
      lspEnabled = !lspEnabled;
      lspButton.setAttribute("aria-pressed", lspEnabled ? "true" : "false");
      lspButton.classList.toggle("btn-primary", lspEnabled);
      if (lspEnabled) {
        scheduleLsp();
      } else {
        renderProblems([]);
        setStatus("LSP off", null, lspStatus);
      }
    });
  }

  if (preset) {
    preset.addEventListener("change", () => loadPreset(preset.value));
  }
  if (runButton) runButton.addEventListener("click", () => run("run"));
  if (checkButton) checkButton.addEventListener("click", () => run("check"));
  if (formatButton) formatButton.addEventListener("click", () => run("format"));

  const saved = localStorage.getItem(STORAGE_KEY);
  editor.value = saved || PRESETS.hello.code;
  renderEditor();
  setOutputTab("console");

  fetch("/api/health")
    .then((response) => response.json())
    .then((data) => {
      if (data.ok) {
        setStatus(data.compiler ? `${data.compiler} · online` : "Compiler online", "is-online");
      } else {
        setStatus("Compiler unavailable", "is-offline");
      }
    })
    .catch(() => {
      setStatus("Playground server offline", "is-offline");
    });

  scheduleLsp();
})();
