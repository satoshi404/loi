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

  // Browsers start the AudioContext suspended. Try to unlock it at load (some
  // desktop setups allow that) and always retry on the first real gesture.
  function ensureAudio() {
    var Ctx = window.AudioContext || window.webkitAudioContext;
    if (!Ctx) return;
    if (!window.loiAudioContext) window.loiAudioContext = new Ctx();
    if (window.loiAudioContext.state === "suspended") {
      var resumed = window.loiAudioContext.resume();
      if (resumed && resumed.catch) resumed.catch(function () {});
    }
  }

  // The GLFW keyboard needs canvas focus, but the synth never requires a key.
  // The first click/tap only serves as the gesture browsers demand for audio.
  function activate() {
    if (hint) hint.style.display = "none";
    if (canvas) canvas.focus();
    ensureAudio();
  }
  document.addEventListener("pointerdown", activate, true);
  document.addEventListener("mousedown", activate, true);
  document.addEventListener("touchstart", activate, true);
  ensureAudio();

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
