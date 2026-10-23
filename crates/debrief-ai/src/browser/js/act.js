// Evaluated in the page by Debrief (see `scripts.rs`); `A` = { nonce, op, ref, tag, text, submit, dir }.
// One agent action; an element is looked up by the ref the snapshot gave it and must still be what Rust
// approved (same tag), and nothing is ever typed into a password field.
function (A) {
  function ret(o) {
    o.nonce = A.nonce;
    o.origin = location.origin;
    o.url = location.href;
    return JSON.stringify(o);
  }
  try {
    if (A.op === 'scroll') {
      window.scrollBy(0, (A.dir === 'up' ? -1 : 1) * Math.round(window.innerHeight * 0.8));
      return ret({ ok: true });
    }
    if (A.op === 'back') {
      history.back();
      return ret({ ok: true });
    }
    if (A.op === 'forward') {
      history.forward();
      return ret({ ok: true });
    }
    var el = document.querySelector('[data-debrief-ref="' + String(A.ref).replace(/[^0-9]/g, '') + '"]');
    if (!el) return ret({ error: 'gone' });
    if (el.tagName.toLowerCase() !== A.tag) return ret({ error: 'changed' });
    el.scrollIntoView({ block: 'center', inline: 'nearest' });
    if (el.focus) el.focus({ preventScroll: true });
    if (A.op === 'click') {
      el.click();
      return ret({ ok: true });
    }
    if (A.op !== 'type') return ret({ error: 'unknown action' });
    if (el.type === 'password') return ret({ error: 'password' });
    if (el.isContentEditable) {
      el.textContent = A.text;
      el.dispatchEvent(new InputEvent('input', { bubbles: true }));
    } else {
      var proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      var desc = Object.getOwnPropertyDescriptor(proto, 'value');
      if (desc && desc.set) desc.set.call(el, A.text);
      else el.value = A.text;
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
    }
    if (A.submit) {
      var key = { key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true, cancelable: true };
      var go = el.dispatchEvent(new KeyboardEvent('keydown', key));
      el.dispatchEvent(new KeyboardEvent('keyup', key));
      if (go && el.form) {
        if (el.form.requestSubmit) el.form.requestSubmit();
        else el.form.submit();
      }
    }
    return ret({ ok: true });
  } catch (e) {
    return ret({ error: String((e && e.message) || e).slice(0, 200) });
  }
}
