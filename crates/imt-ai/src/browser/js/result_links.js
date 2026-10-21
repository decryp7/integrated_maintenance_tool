// Evaluated in the page by IMT (see `scripts.rs`); `A` = { nonce, max, maxText, region }.
// A search results page's links, every `a[href]` in document order (not only the visible ones: the organic results
// sit below the engine's own links), each with its absolute address, its text (a result's heading when it has
// one) and whether it is inside the engine's results region (the first of `A.region` found, else `main`).
// Collects only; Rust decides what is a result (`results.rs`).
function (A) {
  function ret(o) {
    o.nonce = A.nonce;
    o.origin = location.origin;
    o.url = location.href;
    return JSON.stringify(o);
  }
  try {
    var text = function (s, n) {
      return String(s || '').replace(/\s+/g, ' ').trim().slice(0, n);
    };
    var region = null;
    var selectors = (A.region || []).concat(['main', '[role=main]']);
    for (var s = 0; s < selectors.length && !region; s++) {
      try {
        region = document.querySelector(selectors[s]);
      } catch (e) {
        /* not a selector this page understands */
      }
    }
    var all = document.querySelectorAll('a[href]');
    var links = [];
    for (var i = 0; i < all.length && links.length < A.max; i++) {
      var a = all[i];
      var href = a.href;
      if (!href || href.length > 4096) continue;
      var heading = a.querySelector('h3,h2,[role=heading]');
      var label = heading ? heading.textContent : a.textContent || a.getAttribute('aria-label') || a.getAttribute('title') || '';
      links.push({ href: href, text: text(label, A.maxText), region: !!(region && region.contains(a)) });
    }
    var body = document.body ? document.body.innerText || document.body.textContent : '';
    return ret({ title: document.title || '', links: links, total: all.length, regionFound: !!region, text: text(body, 2000) });
  } catch (e) {
    return ret({ error: String((e && e.message) || e).slice(0, 200) });
  }
}
