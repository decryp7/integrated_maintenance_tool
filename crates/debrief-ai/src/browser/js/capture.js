// Evaluated in the page by Debrief (see `scripts.rs`); `A` = { nonce, maxBlocks, maxChars }.
// Lists the visible readable blocks of the page. Scripts, styles, controls and hidden elements are skipped.
function (A) {
  function ret(o) {
    o.nonce = A.nonce;
    o.origin = location.origin;
    o.url = location.href;
    return JSON.stringify(o);
  }
  try {
    var SKIP = { SCRIPT: 1, STYLE: 1, NOSCRIPT: 1, TEMPLATE: 1, SVG: 1, CANVAS: 1, IFRAME: 1, OBJECT: 1, EMBED: 1, HEAD: 1, BUTTON: 1, SELECT: 1, TEXTAREA: 1, INPUT: 1 };
    var KIND = { H1: 'h1', H2: 'h2', H3: 'h3', H4: 'h4', H5: 'h5', H6: 'h6', P: 'p', LI: 'li', PRE: 'pre', BLOCKQUOTE: 'blockquote', TD: 'td', TH: 'th', DT: 'p', DD: 'p', FIGCAPTION: 'p', CAPTION: 'p' };
    var blocks = [];
    var total = 0;
    var visited = 0;
    var truncated = false;
    var visible = function (el) {
      if (el.getAttribute && el.getAttribute('aria-hidden') === 'true') return false;
      if (el.checkVisibility) return el.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true });
      return el.getClientRects().length > 0;
    };
    var add = function (k, t) {
      t = String(t || '').slice(0, 4000);
      if (!t.trim()) return;
      blocks.push({ k: k, t: t });
      total += t.length;
      if (blocks.length >= A.maxBlocks || total >= A.maxChars) truncated = true;
    };
    var walk = function (el, depth) {
      if (truncated || !el || SKIP[el.tagName] || depth > 200) return;
      if (++visited > 50000) {
        truncated = true;
        return;
      }
      if (!visible(el)) return;
      var k = KIND[el.tagName];
      if (k) {
        add(k, el.innerText);
        return;
      }
      var own = '';
      for (var n = el.firstChild; n; n = n.nextSibling) if (n.nodeType === 3) own += n.nodeValue;
      if (own.trim().length > 1) add('p', own.trim());
      for (var c = el.firstElementChild; c; c = c.nextElementSibling) walk(c, depth + 1);
    };
    walk(document.body, 0);
    return ret({ title: document.title || '', blocks: blocks, truncated: truncated });
  } catch (e) {
    return ret({ error: String((e && e.message) || e).slice(0, 200) });
  }
}
