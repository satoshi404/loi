/* Lightweight Loi syntax highlighter shared by docs and the playground. */
(function () {
  "use strict";

  const KEYWORDS = new Set([
    "shape", "flow", "open", "sealed", "mut", "in", "give", "clone",
    "protect", "alloc", "import", "comptime", "if", "else", "while",
    "for", "continue", "break", "and", "or", "not", "as", "true", "false",
  ]);

  const TYPES = new Set([
    "i8", "i16", "i32", "i64", "i128",
    "u8", "u16", "u32", "u64", "u128",
    "f32", "f64", "bool", "void",
  ]);

  const TOKEN = /(\/\/[^\n]*)|("(?:\\.|[^"\\])*")|(\b(?:0[xX][0-9a-fA-F_]+|0[bB][01_]+|\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d+)?)\b)|([A-Za-z_][A-Za-z0-9_]*)|(<-|<->|->|=>|\.\.|==|!=|<=|>=|\+=|-=|\*=|\/=)/g;

  function escapeHtml(text) {
    return text
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function highlight(source) {
    let html = "";
    let last = 0;
    let match;

    TOKEN.lastIndex = 0;
    while ((match = TOKEN.exec(source)) !== null) {
      const text = match[0];
      html += escapeHtml(source.slice(last, match.index));

      let kind = "";
      if (match[1]) {
        kind = "comment";
      } else if (match[2]) {
        kind = "string";
      } else if (match[3]) {
        kind = "number";
      } else if (match[4]) {
        if (KEYWORDS.has(text)) {
          kind = "keyword";
        } else if (TYPES.has(text)) {
          kind = "type";
        } else {
          const rest = source.slice(match.index + text.length);
          if (/^\s*\(/.test(rest)) {
            kind = "fn";
          } else if (/^[A-Z]/.test(text)) {
            kind = "type";
          }
        }
      } else if (match[5]) {
        kind = "op";
      }

      html += kind
        ? `<span class="tok-${kind}">${escapeHtml(text)}</span>`
        : escapeHtml(text);
      last = match.index + text.length;
    }

    html += escapeHtml(source.slice(last));
    return html;
  }

  window.LoiHighlight = { highlight, escapeHtml };
})();
