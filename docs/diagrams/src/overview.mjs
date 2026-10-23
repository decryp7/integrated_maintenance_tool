// Debrief's overview SVG (debrief-overview.svg), drawn with the Paddock kit's palette: one file, theme-aware through a
// prefers-color-scheme style block. Usage: node docs/diagrams/src/overview.mjs
import { theme } from '../../../../paddock/diagrams/tools/themes.mjs';
import { tw, wrap } from '../../../../paddock/diagrams/tools/lib.mjs';
import { writeFileSync } from 'node:fs';
import { OUT, DATE } from './common.mjs';

const t = theme('debrief');
const FONT = '-apple-system,BlinkMacSystemFont,\'Segoe UI\',system-ui,sans-serif';
const MONO = 'ui-monospace,\'SF Mono\',\'Cascadia Mono\',Consolas,monospace';
const roles = { accent: 'acc', ui: 'ui', core: 'core', plugin: 'plg', process: 'pro', external: 'ext', group: 'grp' };
const vars = (i) => Object.entries(roles).map(([r, n]) => `--${n}-s:${t[r].s[i]};--${n}-f:${t[r].f[i]};`).join('') + `--ink:${t.text[i]};--mut:${t.muted[i]};--line:${t.edge[i]};`;
const out = [];
const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
const W = 1280;

/** Text with `code` runs in monospace. */
function runs(x, y, s, cls) {
  const parts = s.split('`').map((p, i) => (i % 2 ? `<tspan class="c">${esc(p)}</tspan>` : esc(p))).join('');
  out.push(`<text x="${x}" y="${y}" class="${cls}">${parts}</text>`);
}
function box(x, y, w, role, name, body) {
  const lines = wrap(body.replace(/`/g, ''), 12, w - 32);
  const h = 22 + 20 + lines.length * 16 + 18;
  out.push(`<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="6" class="n ${roles[role]}"/>`);
  runs(x + 16, y + 30, name, 'h');
  // re-wrap with the code markers kept: wrap the plain text, then re-mark by position is fragile, so wrap the marked text by words
  const marked = wrapMarked(body, w - 32);
  marked.forEach((l, i) => runs(x + 16, y + 52 + i * 16, l, 't'));
  return h;
}
function wrapMarked(text, maxw) {
  const words = text.split(/ +/);
  const lines = [];
  let line = '';
  let open = false;
  for (const wd of words) {
    const cand = line ? `${line} ${wd}` : wd;
    if (line && tw(cand.replace(/`/g, ''), 12) > maxw) { lines.push(line); line = wd; } else line = cand;
  }
  if (line) lines.push(line);
  // a code run split across lines: close and reopen the backticks
  return lines.map((l) => { let s = l; if (open) s = '`' + s; const n = (l.match(/`/g) || []).length; open = (open ? n + 1 : n) % 2 === 1; if (open) s += '`'; return s; });
}
function group(x, y, w, h, label) {
  out.push(`<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="10" class="g"/>`);
  out.push(`<text x="${x + 16}" y="${y + 28}" class="gl">${esc(label)}</text>`);
}
function arrow(x1, y1, x2, y2, label) {
  out.push(`<path d="M${x1} ${y1} L${x2} ${y2}" class="e" marker-end="url(#a)"/>`);
  const w = tw(label, 12) + 4;
  out.push(`<text x="${Math.round((x1 + x2) / 2 - w / 2)}" y="${y1 - 10}" class="m">${esc(label)}</text>`);
}

out.push(`<text x="48" y="64" class="title">Debrief</text>`);
out.push(`<text x="48" y="94" class="sub">An IDE for analysing logs and diagnostic files: import once, correlate on one timeline, explain and share</text>`);
out.push(`<text x="48" y="118" class="desc">Local-first · a Rust engine, sandboxed plugins and an Angular workbench · Windows, macOS and Linux</text>`);

const OX = 48, OW = 288, EX = OX + OW + 112, EW = 384, AX = EX + EW + 112, AW = 288, top = 160;
const GAP = 24, y0 = top + 48;
// outside
let y = y0;
const col = (x, w, items) => { let cy = y0; const ys = []; for (const [role, name, body] of items) { const h = box(x, cy, w, role, name, body); ys.push([cy, h]); cy += h + GAP; } return { end: cy - GAP, ys }; };
const o = col(OX + 24, OW - 48, [
  ['external', 'Logs and archives', 'Serilog CLEF, JSON, text, log4net, NLog, log4j, CSV, syslog and journald, IIS and web servers, HAR, OpenTelemetry, Docker and Kubernetes, `.zip`, `.gz` and `.debriefz` bundles.'],
  ['external', 'Windows diagnostics', 'Event logs, Performance Monitor, Process Monitor, registry exports, msinfo, crash dumps and UMDH heap logs.'],
  ['external', 'Captures and recordings', 'Profiler captures (speedscope, pprof, perf, nettrace, Chrome), heap snapshots and screen recordings.'],
  ['accent', 'Collection and automation', '`debrief-agent`, `debrief-collect`, `debrief-monitor`, the `debrief` command line for scripts and CI, and watched folders.'],
]);
const e = col(EX + 24, EW - 48, [
  ['plugin', 'Plugins: importers, tools, transforms', 'Each in its own sandboxed process over one IPC (`debrief-ipc`): built in, or signed `.debriefplugin` packages written in Rust.'],
  ['accent', 'Store worker', 'DuckDB and Tantivy in a supervised process: Parquet segments, a full-text index, time-range import and `.debriefz` projects. The app never holds the records.'],
  ['accent', 'Queries and analysis', 'Query language, facets, paging and charts; anomalies, patterns, exceptions, error outliers, Moment, known issues and incident reports.'],
  ['accent', 'AI and research', 'A local `llama-server` worker with vision and knowledge files; research and Agent mode drive a real Chromium over DevTools.'],
]);
const a = col(AX + 24, AW - 48, [
  ['ui', 'Investigate', 'A timeline per instance, the events grid and inspector, lenses and filter sets, search everything and Home.'],
  ['ui', 'Follow', 'Live tail with alerts, watched folders, the Jobs pane and notifications.'],
  ['ui', 'Teach and share', 'Profiles and parsers, reports, redacted copies, bundles and notes.'],
  ['accent', 'Tauri shell', 'Validates every request and codes against the Store ports only; least-privilege capabilities.'],
]);
const bodyEnd = Math.max(o.end, e.end, a.end) + 24;
const ph = bodyEnd - top;
group(OX, top, OW, ph, 'Outside Debrief');
group(EX, top, EW, ph, 'Engine and workers: compute everything');
group(AX, top, AW, ph, 'Workbench: show and control');
// re-draw boxes on top of the groups: groups were pushed after the boxes, so move them to the front of the list
const groupsXml = out.splice(out.length - 6, 6);
const firstBox = out.findIndex((s) => s.startsWith('<rect'));
out.splice(firstBox, 0, ...groupsXml);
// arrows in the gutters, between the panels
const gy = (n) => top + 48 + 90 + n * 150;
arrow(OX + OW, o.ys[0][0] + 40, EX, o.ys[0][0] + 40, 'read by');
arrow(EX + EW, e.ys[2][0] + 40, AX, e.ys[2][0] + 40, 'results');
arrow(AX, e.ys[3][0] + 40, EX + EW, e.ys[3][0] + 40, 'requests');
arrow(OX + OW, o.ys[3][0] + 40, EX, o.ys[3][0] + 40, 'writes bundles');
// core bar
const cy = bodyEnd + 40;
const coreH = 112;
out.push(`<rect x="48" y="${cy}" width="1184" height="112" rx="6" class="n core"/>`);
out.push(`<text x="64" y="${cy + 30}" class="h">debrief-core: pure Rust, no windows, no network</text>`);
['Store ports, parsers and profiles, the pipeline, the query language, analysis, reports and redaction: the single source of truth for data and behaviour. The app, the store worker and `debrief` all use it.', 'Files are untrusted: parsed once at the edge, bounded by line, regex and archive limits, fuzzed, and never allowed to panic. Nothing leaves your machine unless you start an update, a download or research.'].forEach((p, i) => wrapMarked(p, 1152).forEach((l, k) => runs(64, cy + 54 + (i * 2 + k) * 16 - (i ? 0 : 0), l, 't')));
// legend
let lx = 64; const ly = cy + 112 + 56;
const legW = [['Outside'], ['Plugin'], ['Debrief'], ['Workbench'], ['Pure core']].reduce((a, [l]) => a + 40 + tw(l, 12) + 36, 0) - 36 + 32;
out.push(`<rect x="48" y="${ly - 20}" width="${legW}" height="40" rx="14" class="g"/>`);
for (const [r, label] of [['external', 'Outside'], ['plugin', 'Plugin'], ['accent', 'Debrief'], ['ui', 'Workbench'], ['core', 'Pure core']]) {
  out.push(`<rect x="${lx + 4}" y="${ly - 8}" width="28" height="16" rx="4" class="n ${roles[r]}"/>`);
  out.push(`<text x="${lx + 40}" y="${ly + 4}" class="m">${esc(label)}</text>`);
  lx += 40 + tw(label, 12) + 36;
}
out.push(`<text x="1232" y="${ly + 4}" class="foot">Debrief · updated ${DATE}</text>`);
const H = ly + 48;

const css = `
    :root{${vars(0)}}
    @media (prefers-color-scheme: dark){:root{${vars(1)}}}
    text{font-family:${FONT};fill:var(--ink)}
    .c{font-family:${MONO}}
    .title{font-size:24px;font-weight:600}.sub{font-size:14px;fill:var(--mut)}.desc{font-size:12px;fill:var(--mut)}
    .h{font-size:14px;font-weight:600}.t{font-size:12px}.m{font-size:12px;fill:var(--mut)}.gl{font-size:12px;font-weight:600;fill:var(--mut)}.foot{font-size:11px;fill:var(--mut);text-anchor:end}
    .g{fill:var(--grp-f);stroke:var(--grp-s);stroke-width:1}
    .n{stroke-width:1.5}
    .acc{fill:var(--acc-f);stroke:var(--acc-s)}.ui{fill:var(--ui-f);stroke:var(--ui-s)}.core{fill:var(--core-f);stroke:var(--core-s)}.plg{fill:var(--plg-f);stroke:var(--plg-s)}.pro{fill:var(--pro-f);stroke:var(--pro-s)}
    .ext{fill:var(--ext-f);stroke:var(--ext-s);stroke-dasharray:6 4}
    .e{stroke:var(--line);stroke-width:1.5;fill:none}.ah{fill:var(--line)}`;
const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" role="img" aria-labelledby="t d">
  <title id="t">Debrief architecture overview</title>
  <desc id="d">Logs, Windows diagnostics, profiler captures and recordings, and collection tools feed importers that run as sandboxed plugins. A Rust engine imports them into a DuckDB store worker, answers queries and analyses, and drives an optional local AI and a research browser. The Angular and Tauri workbench shows and controls it. A pure Rust core holds all the rules.</desc>
  <defs><marker id="a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path d="M0 0 L10 5 L0 10 z" class="ah"/></marker></defs>
  <style>${css}
  </style>
${out.join('\n')}
</svg>
`;
writeFileSync(`${OUT}/debrief-overview.svg`, svg);
