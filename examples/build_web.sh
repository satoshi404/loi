#!/usr/bin/env sh
# Compila um exemplo de raylib para a web (WebAssembly) com Emscripten.
#
# Pre-requisitos:
#   1) Emscripten ativo no shell (emcc no PATH, ex.: source emsdk_env.sh)
#   2) raylib compilada para web (NAO e a libraylib do sistema):
#        cd raylib-5.5/src && make PLATFORM=PLATFORM_WEB
#      e RAYLIB_WEB apontando para essa pasta src
#
# Uso (da raiz do repositorio):
#   RAYLIB_WEB=/caminho/raylib-5.5/src sh examples/build_web.sh examples/raylib_web.loi
#   RAYLIB_WEB=/caminho/raylib-5.5/src WEB_C_SOURCES=examples/sound_shim.c \
#       sh examples/build_web.sh examples/sound_synth.loi
#   python3 -m http.server -d web 8000   # abra http://localhost:8000
#
# Variaveis opcionais:
#   LOI_BIN         caminho do compilador (padrao: loi)
#   OUT_DIR         pasta de saida (padrao: web)
#   WEB_C_SOURCES   fontes C adicionais separados por espaco (ex.: sound_shim.c)
set -eu

SOURCE=${1:-examples/raylib_web.loi}
OUT_DIR=${OUT_DIR:-web}
LOI_BIN=${LOI_BIN:-loi}
: "${RAYLIB_WEB:?defina RAYLIB_WEB com a pasta src do raylib compilado para web}"

mkdir -p "$OUT_DIR"
NAME=$(basename "$SOURCE" .loi)

EXTRA_SOURCES=""
for src in ${WEB_C_SOURCES:-}; do
    EXTRA_SOURCES="$EXTRA_SOURCES --c-source $src"
done

# CC=emcc faz o loi chamar o Emscripten no lugar do cc.
# -sASYNCIFY      deixa o 'while not WindowShouldClose()' bloqueante funcionar no navegador
# -sUSE_GLFW=3    janela/entrada pelo GLFW do Emscripten
# -sALLOW_MEMORY_GROWTH  evita estourar a memoria inicial
# Geramos apenas o .js/.wasm e escrevemos um HTML proprio: o HTML padrao do
# Emscripten usa scripts inline, bloqueados pela CSP do site.
# shellcheck disable=SC2086
CC=emcc "$LOI_BIN" build "$SOURCE" -o "$OUT_DIR/$NAME.js" --release \
    --c-source vendor/raylib/raylib_shim.c $EXTRA_SOURCES \
    --cc-flag -Ivendor/raylib/include \
    --cc-flag "-L$RAYLIB_WEB" \
    --cc-flag -lraylib \
    --cc-flag -DPLATFORM_WEB \
    --cc-flag -sUSE_GLFW=3 \
    --cc-flag -sASYNCIFY \
    --cc-flag -sALLOW_MEMORY_GROWTH=1 \
    --cc-flag -sENVIRONMENT=web

# Remove o stamp do loi: ele e util para builds locais, mas nao faz parte do site.
rm -f "$OUT_DIR/$NAME.js.loi-stamp"

cat > "$OUT_DIR/$NAME.loader.js" <<'EOF'
(function () {
  "use strict";
  var canvas = document.getElementById("canvas");
  var status = document.getElementById("status");
  var output = document.getElementById("output");
  var hint = document.getElementById("hint");

  function write(target, args) {
    if (!target) return;
    target.textContent += args.join(" ") + "\n";
  }

  // O teclado do Emscripten so chega ao GLFW quando o canvas tem foco. O
  // primeiro clique tambem libera o AudioContext suspenso do WebAudio.
  function activate() {
    if (hint) hint.style.display = "none";
    if (canvas) canvas.focus();
  }
  document.addEventListener("pointerdown", activate, true);
  document.addEventListener("mousedown", activate, true);
  document.addEventListener("touchstart", activate, true);

  window.Module = {
    canvas: canvas,
    print: function () { write(output, Array.prototype.slice.call(arguments)); },
    printErr: function () { write(output, Array.prototype.slice.call(arguments)); },
    setStatus: function (text) {
      if (status) status.textContent = text || "Pronto.";
      if (!text && output) output.textContent = "";
    },
    monitorRunDependencies: function (left) {
      if (status) status.textContent = left ? "Preparando... (" + left + ")" : "Carregado.";
    }
  };
})();
EOF

cat > "$OUT_DIR/$NAME.html" <<EOF
<!doctype html>
<html lang="pt-br">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Loi + raylib na web</title>
  <style>
    :root { color-scheme: dark; }
    body {
      margin: 0;
      padding: 12px;
      background: #0a0e14;
      color: #e8ecf8;
      font: 13px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace;
    }
    #status { margin-bottom: 8px; color: #98a2b8; }
    #stage { position: relative; display: inline-block; max-width: 100%; }
    #hint {
      position: absolute;
      inset: 0;
      display: flex;
      align-items: center;
      justify-content: center;
      padding: 16px;
      background: rgba(10, 14, 20, 0.78);
      color: #e8ecf8;
      text-align: center;
      cursor: pointer;
    }
    #canvas {
      display: block;
      max-width: 100%;
      border: 1px solid #2a3040;
      background: #000;
    }
    #output {
      display: none;
      max-height: 96px;
      overflow: auto;
      margin: 8px 0 0;
      color: #98a2b8;
      white-space: pre-wrap;
    }
  </style>
</head>
<body>
  <div id="status">Carregando...</div>
  <div id="stage">
    <canvas id="canvas" tabindex="0"></canvas>
    <div id="hint">Clique para focar o teclado e liberar o audio</div>
  </div>
  <pre id="output"></pre>
  <script src="$NAME.loader.js"></script>
  <script async src="$NAME.js"></script>
</body>
</html>
EOF

echo "Pronto: $OUT_DIR/$NAME.html"
