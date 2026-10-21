// Evaluated in the page by IMT (see `scripts.rs`); `A` = { nonce, max, maxText, region, slot }.
// A search results page's links, every `a[href]` in document order (not only the visible ones: the organic results
// sit below the engine's own links), each with its absolute address, its text (a result's heading when it has
// one, and whether it has one), and whether it is inside the engine's results region (the first of `A.region` found, else `main`).
// Also how ready the page is (`readiness.rs`): its readyState and how long its DOM has been quiet, measured by a
// MutationObserver the first call on a document installs in a non-enumerable property named `A.slot` (a random name
// per IMT process, so the page cannot know it in advance; nothing else is added to the page). Synchronous on purpose:
// the webview does not wait for promises.
// Collects only; Rust decides what is a result (`results.rs`).
function (A) {
  function ret(o) {
    o.nonce = A.nonce;
    o.origin = location.origin;
    o.url = location.href;
    return JSON.stringify(o);
  }
  // Milliseconds since the DOM last changed; 0 on the first call (the observer starts now), null without an observer.
  function quiet() {
    var now = performance.now();
    var st = null;
    try {
      st = Object.prototype.hasOwnProperty.call(document, A.slot) ? document[A.slot] : null;
      if (!st) {
        var box = { last: now };
        new MutationObserver(function () {
          box.last = performance.now();
        }).observe(document.documentElement || document, { childList: true, subtree: true });
        Object.defineProperty(document, A.slot, { value: box, enumerable: false, configurable: false, writable: false });
        st = box;
      }
    } catch (e) {
      return null;
    }
    return st && typeof st.last === 'number' ? Math.max(0, Math.round(now - st.last)) : null;
  }
  try {
    var quietMs = quiet();
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
      links.push({ href: href, text: text(label, A.maxText), region: !!(region && region.contains(a)), heading: !!heading });
    }
    var body = document.body ? document.body.innerText || document.body.textContent : '';
    return ret({
      title: document.title || '',
      links: links,
      total: all.length,
      regionFound: !!region,
      text: text(body, 2000),
      readyState: document.readyState,
      quietMs: quietMs,
    });
  } catch (e) {
    return ret({ error: String((e && e.message) || e).slice(0, 200) });
  }
}
