// loi site script
(function () {
  const KEYWORDS = new Set([
    'static','const','let','struct','union','enum','type','fn','region',
    'return','if','else','while','for','do','switch','case','default','break',
    'continue','main','extern','asm','sizeof','typeof','defined','asm'
  ]);
  const TYPES = new Set(['int','float','double','char','bool','byte','short','long','void','unsigned','ptr']);
  const TOKEN_RE = /("(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|\/\/[^\n]*|\/\*[\s\S]*?\*\/|@[a-zA-Z_]\w*|@\(|\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b|[A-Za-z_]\w*|[^A-Za-z0-9_"'@]/g;

  function esc(s) { return s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;'); }
  function cls(tok) {
    if (tok.startsWith('//') || tok.startsWith('/*')) return 'tok-comment';
    if (tok.startsWith('"')) return 'tok-str';
    if (tok.startsWith("'")) return 'tok-char';
    if (tok.startsWith('@')) return 'tok-directive';
    if (/^\d/.test(tok)) return 'tok-num';
    if (KEYWORDS.has(tok)) return 'tok-kw';
    if (TYPES.has(tok)) return 'tok-type';
    if (/^[A-Za-z_]\w*$/.test(tok)) return '';
    return 'tok-punct';
  }
  function highlight(code) {
    return code.replace(TOKEN_RE, (m) => {
      const c = cls(m);
      const e = esc(m);
      return c ? '<span class="' + c + '">' + e + '</span>' : e;
    });
  }

  // Apply to code blocks tagged <code class="lang-loi">
  document.addEventListener('DOMContentLoaded', () => {
    document.querySelectorAll('pre code').forEach((el) => {
      const raw = el.textContent;
      el.innerHTML = highlight(raw);
    });
  });

  // Mobile nav toggle (if needed)
  const btn = document.getElementById('navtoggle');
  if (btn) btn.addEventListener('click', () => {
    document.querySelector('nav.main-nav')?.classList.toggle('open');
  });
})();