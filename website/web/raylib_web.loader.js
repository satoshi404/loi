(function () {
  "use strict";
  var canvas = document.getElementById("canvas");
  var status = document.getElementById("status");
  var output = document.getElementById("output");

  function write(target, args) {
    if (!target) return;
    target.textContent += args.join(" ") + "\n";
  }

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
