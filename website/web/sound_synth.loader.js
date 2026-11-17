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
    // Browsers create the AudioContext suspended; the first click is a user
    // gesture, so resume it here before the program reaches ESPAÇO.
    var Ctx = window.AudioContext || window.webkitAudioContext;
    if (!Ctx) return;
    if (!window.loiAudioContext) window.loiAudioContext = new Ctx();
    if (window.loiAudioContext.state === "suspended") {
      window.loiAudioContext.resume();
    }
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
