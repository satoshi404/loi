/* Loi playground: editor, syntax overlay and compiler API client. */
(function () {
  "use strict";

  const editor = document.getElementById("editor");
  const highlight = document.getElementById("highlight");
  const gutter = document.getElementById("gutter");
  const output = document.getElementById("output");
  const meta = document.getElementById("output-meta");
  const status = document.getElementById("playground-status");
  const preset = document.getElementById("preset");
  const runButton = document.querySelector('[data-action="run"]');
  const checkButton = document.querySelector('[data-action="check"]');
  const formatButton = document.querySelector('[data-action="format"]');
  if (!editor || !highlight || !gutter || !output) return;

  const STORAGE_KEY = "loi-playground-source";
  const MAX_LENGTH = 200_000;

  const PRESETS = {
    hello: {
      label: "Hello, Loi",
      code: `shape Greeting [
    message: [u8]
    times:   u32
]

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
    greet(Greeting[ message: "Hello, Loi!\\n", times: 3 ])
    print("The playground runs the real compiler.\\n")
    => 0
}
`,
    },
    directions: {
      label: "Directions & flows",
      code: `shape Counter [ value: i32 ]

sealed flow bump
    <-> counter: Counter
    <- amount: i32
{
    counter.value += amount
}

sealed flow classify
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
    mut counter: Counter <- Counter[ value: 0 ]
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
      code: `shape Point [ x: i32, y: i32 ]

open flow make_point
    -> point: Point
{
    point = Point[ x: 10, y: 20 ]
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
  };

  let busy = false;

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

  function setStatus(text, state) {
    if (!status) return;
    status.classList.remove("is-online", "is-offline");
    if (state) status.classList.add(state);
    const dot = status.querySelector("i");
    if (dot) dot.setAttribute("aria-hidden", "true");
    const label = status.querySelector("span");
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
    details.textContent = [stage, exit, formatDuration(result.durationMs)]
      .filter(Boolean)
      .join(" · ");
    meta.appendChild(details);
  }

  async function api(path, source) {
    const response = await fetch(path, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ source }),
    });

    let payload;
    try {
      payload = await response.json();
    } catch (_) {
      throw new Error(`The playground server returned HTTP ${response.status} without JSON.`);
    }

    if (!response.ok && payload.error) {
      throw new Error(payload.error);
    }
    return payload;
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
    showOutput(mode === "format" ? "Formatting…" : "Compiling…");
    if (meta) meta.textContent = "";

    try {
      const result = await api(`/api/${mode}`, source);

      if (mode === "format" && result.ok) {
        editor.value = result.formatted;
        localStorage.setItem(STORAGE_KEY, result.formatted);
        renderEditor();
        showOutput("Formatted with the canonical Flow Syntax formatter.\n");
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
          "Could not load the tic-tac-toe example. Start the playground server to load it from the website examples directory.",
          "error"
        );
        return;
      }
    } else {
      editor.value = item.code;
    }
    localStorage.setItem(STORAGE_KEY, editor.value);
    renderEditor();
    showOutput("Loaded preset. Press Run to compile and execute it.");
    if (meta) meta.textContent = "";
  }

  editor.addEventListener("input", () => {
    localStorage.setItem(STORAGE_KEY, editor.value);
    renderEditor();
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

  if (preset) {
    preset.addEventListener("change", () => loadPreset(preset.value));
  }
  if (runButton) runButton.addEventListener("click", () => run("run"));
  if (checkButton) checkButton.addEventListener("click", () => run("check"));
  if (formatButton) formatButton.addEventListener("click", () => run("format"));

  const saved = localStorage.getItem(STORAGE_KEY);
  editor.value = saved || PRESETS.hello.code;
  renderEditor();

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
})();
