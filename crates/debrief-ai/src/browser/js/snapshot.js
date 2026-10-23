// Evaluated in the page by Debrief (see `scripts.rs`); `A` = { nonce, maxElements }.
// Numbers the interactive elements (`data-debrief-ref`, stable while the page lives), visible ones first, and
// collects the raw facts Rust classifies: input fields, frame addresses, CAPTCHA-like class and id names.
function (A) {
  function ret(o) {
    o.nonce = A.nonce;
    o.origin = location.origin;
    o.url = location.href;
    return JSON.stringify(o);
  }
  try {
    var SEL = 'a[href],button,input,select,textarea,summary,[role=button],[role=link],[role=checkbox],[role=radio],[role=switch],[role=tab],[role=menuitem],[role=textbox],[role=searchbox],[role=combobox],[contenteditable=true]';
    var BOILER = 'nav,footer,aside,header,[role=navigation],[role=banner],[role=contentinfo]';
    var vh = window.innerHeight;
    var vw = window.innerWidth;
    var shown = function (el) {
      if (el.getAttribute('aria-hidden') === 'true') return false;
      if (el.checkVisibility && !el.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true })) return false;
      var r = el.getBoundingClientRect();
      return r.width > 0 || r.height > 0;
    };
    var text = function (s, n) {
      return String(s || '').replace(/\s+/g, ' ').trim().slice(0, n);
    };
    var nameOf = function (el) {
      var label = el.getAttribute('aria-label') || '';
      if (!label && el.labels && el.labels.length) label = el.labels[0].innerText;
      if (!label && el.tagName !== 'INPUT' && el.tagName !== 'TEXTAREA' && el.tagName !== 'SELECT') label = el.innerText;
      if (!label && el.tagName === 'INPUT' && (el.type === 'submit' || el.type === 'button' || el.type === 'reset')) label = el.value;
      if (!label) label = el.getAttribute('placeholder') || el.getAttribute('title') || el.getAttribute('alt') || el.getAttribute('name') || '';
      return text(label, 160);
    };
    var all = document.querySelectorAll(SEL);
    var max = 0;
    var i;
    for (i = 0; i < all.length; i++) {
      var had = parseInt(all[i].getAttribute('data-debrief-ref') || '0', 10);
      if (had > max) max = had;
    }
    var out = [];
    for (i = 0; i < all.length && i < 3000; i++) {
      var el = all[i];
      if (el.disabled || (el.tagName === 'INPUT' && el.type === 'hidden') || !shown(el)) continue;
      var rect = el.getBoundingClientRect();
      var inView = rect.bottom > 0 && rect.right > 0 && rect.top < vh && rect.left < vw;
      if (!inView && el.closest(BOILER)) continue;
      var ref = el.getAttribute('data-debrief-ref');
      if (!ref) {
        ref = String(++max);
        el.setAttribute('data-debrief-ref', ref);
      }
      var form = el.form || el.closest('form');
      var tag = el.tagName.toLowerCase();
      var type = (el.getAttribute('type') || '').toLowerCase();
      out.push({
        r: parseInt(ref, 10),
        tag: tag,
        type: type,
        role: (el.getAttribute('role') || '').toLowerCase(),
        name: nameOf(el),
        href: tag === 'a' ? el.href : '',
        form: !!form,
        submit: (tag === 'button' && !!form && (type === '' || type === 'submit')) || (tag === 'input' && (type === 'submit' || type === 'image')),
        vis: inView,
      });
    }
    out.sort(function (a, b) {
      return (b.vis ? 1 : 0) - (a.vis ? 1 : 0) || a.r - b.r;
    });
    out = out.slice(0, A.maxElements);
    var fields = [];
    var inputs = document.querySelectorAll('input,textarea,select');
    for (i = 0; i < inputs.length && fields.length < 200; i++) {
      var f = inputs[i];
      fields.push({
        type: (f.getAttribute('type') || f.tagName).toLowerCase(),
        ac: (f.getAttribute('autocomplete') || '').toLowerCase(),
        nm: text([f.getAttribute('name'), f.id, f.getAttribute('placeholder'), f.getAttribute('aria-label')].join(' '), 120),
        vis: f.type !== 'hidden' && shown(f),
      });
    }
    var frames = [];
    var iframes = document.querySelectorAll('iframe[src]');
    for (i = 0; i < iframes.length && frames.length < 50; i++) {
      try {
        var u = new URL(iframes[i].getAttribute('src'), location.href);
        frames.push((u.host + u.pathname).slice(0, 200));
      } catch (e) {
        /* not an address */
      }
    }
    var marks = [];
    var odd = document.querySelectorAll('[class*=captcha i],[id*=captcha i],[class*=turnstile i],[class*=challenge i],[id*=challenge i]');
    for (i = 0; i < odd.length && marks.length < 40; i++) marks.push(text((odd[i].getAttribute('class') || '') + ' ' + (odd[i].id || ''), 200));
    var main = document.querySelector('main,[role=main],article') || document.body;
    return ret({ title: document.title || '', elements: out, fields: fields, frames: frames, marks: marks, text: text(main ? main.innerText : '', 6000) });
  } catch (e) {
    return ret({ error: String((e && e.message) || e).slice(0, 200) });
  }
}
