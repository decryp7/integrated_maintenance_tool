// The drag ghost's page (see drag-ghost.html). The shell calls `window.debriefGhost(content)` with
// `{ title, icon, snapshot, dark }`: text goes in with textContent, the icon is an SVG path's `d`, the snapshot a PNG
// data URL; nothing is ever parsed as markup.
(function () {
  'use strict';
  var root = document.documentElement;
  var title = document.querySelector('.title');
  var icon = document.querySelector('.icon');
  var path = document.querySelector('.icon path');
  var snapshot = document.querySelector('.snapshot');
  window.debriefGhost = function (c) {
    if (!c || typeof c !== 'object') return;
    root.setAttribute('data-scheme', c.dark ? 'dark' : 'light');
    title.textContent = typeof c.title === 'string' ? c.title : '';
    if (typeof c.icon === 'string' && c.icon) {
      path.setAttribute('d', c.icon);
      icon.setAttribute('data-on', '');
    } else {
      path.removeAttribute('d');
      icon.removeAttribute('data-on');
    }
    if (typeof c.snapshot === 'string' && c.snapshot.indexOf('data:image/png;base64,') === 0) {
      snapshot.src = c.snapshot;
      snapshot.hidden = false;
    } else {
      snapshot.removeAttribute('src');
      snapshot.hidden = true;
    }
  };
})();
