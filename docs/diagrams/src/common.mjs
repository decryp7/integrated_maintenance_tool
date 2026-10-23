// Shared helpers for the Debrief diagram generators. The Paddock diagram kit is expected as a sibling checkout
// (../paddock next to the debrief checkout); see README.md in this folder.
import { Diagram, link, g8, tw, wrap } from '../../../../paddock/diagrams/tools/lib.mjs';
import { sequence, dot, cls } from '../../../../paddock/diagrams/tools/uml.mjs';
import { writeFileSync } from 'node:fs';
export { link, g8, tw, wrap, sequence, dot, cls };

export const OUT = new URL('..', import.meta.url).pathname.replace(/\/$/, '');
export const DATE = '2026-10-23';
export const P = (o) => new Diagram({ app: 'debrief', date: DATE, ...o });
export const save = (name, d, bottom) => writeFileSync(`${OUT}/${name}.drawio`, d.finish(bottom));
export const Y0 = 144;

/** Port fraction on rect `hub` for the centre of rect `other` (for straight links from a tall hub). */
export function tOf(d, hub, other, axis = 1) {
  const [x, y, w, h] = d.rect[hub], [ox, oy, ow, oh] = d.rect[other];
  return axis === 1 ? (oy + oh / 2 - y) / h : (ox + ow / 2 - x) / w;
}
/** Straight link from a tall hub to a node beside it; rev: the arrow points at the hub. */
export function hub(d, h, sh, n, sn, opts = {}) {
  const t = tOf(d, h, n, sh === 'T' || sh === 'B' ? 0 : 1);
  const [a, b] = opts.rev ? [n, h] : [h, n];
  const [sa, sb] = opts.rev ? [sn, sh] : [sh, sn];
  const ta = opts.rev ? 0.5 : t, tb = opts.rev ? t : 0.5;
  const rest = { ...opts }; delete rest.rev;
  return link(d, a, sa, b, sb, { ...rest, t: [ta, tb] });
}
/** Stack nodes in a column, all `w` wide; returns ids. items: [id, role, { name, tech, desc }, h?]. Gap 40 unless given. */
export function stack(d, x, y, w, items, gap = 40) {
  let cy = y;
  for (const [id, role, o, h] of items) {
    d.node(id, x, cy, w, h ?? null, role, o);
    cy += d.rect[id][3] + gap;
  }
  return cy - gap;
}
/** Nodes of one row share the tallest height. items: [id, role, opts]; returns the row height. */
export function row(d, x, y, w, gap, items, minH = 56) {
  const h = Math.max(minH, ...items.map(([, , o]) => d.fit(w, o)));
  items.forEach(([id, role, o], i) => d.node(id, x + i * (w + gap), y, w, h, role, o));
  return h;
}
export const bottom = (d, ...ids) => Math.max(...ids.map((i) => d.rect[i][1] + d.rect[i][3]));
export const right = (d, ...ids) => Math.max(...ids.map((i) => d.rect[i][0] + d.rect[i][2]));
export const NAMES = { accent: 'Debrief', core: 'Rust engine (pure)', ui: 'Angular UI', plugin: 'Plugin', process: 'Process · OS · data', external: 'External system' };
/** Legend entries for the given roles plus line styles. */
export const legend = (d, roles, lines = ['line'], names = {}) => d.legend([
  ...roles.map((k) => ({ kind: k, label: names[k] ?? NAMES[k] })),
  ...lines.map((k) => ({ kind: k, label: k === 'line' ? 'Uses' : k === 'dashline' ? 'Reply or dependency' : k })),
]);

/**
 * A boundary group holding a column of nodes (inner width `nw`). The frame is drawn first (draw.io z-order is file order).
 * nodes: [id, role, { name, tech, desc }, h?]. Returns the group's [x, y, w, h]; node and group ids are registered as rects.
 */
export function grp(d, id, x, y, label, nodes, { nw = 232, os = false, minH = 0, gap = 40 } = {}) {
  const hs = nodes.map(([, , o, h]) => h ?? d.fit(nw, o));
  const need = 40 + hs.reduce((a, b) => a + b, 0) + gap * (nodes.length - 1) + 24;
  const h = Math.max(minH, need);
  if (minH > need) hs[hs.length - 1] += minH - need; // the last node stretches to fill the group
  d.group(id, x, y, nw + 48, h, label, { os });
  let cy = y + 40;
  nodes.forEach(([nid, role, o], i) => { d.node(nid, x + 24, cy, nw, hs[i], role, o); cy += hs[i] + gap; });
  return [x, y, nw + 48, h];
}

/** Restyles an edge already added (by the id link() returned): fn gets the style string. */
export function restyle(d, id, fn) {
  const i = d.cells.findIndex((c) => c.startsWith(`<mxCell id="${id}"`));
  if (i >= 0) d.cells[i] = d.cells[i].replace(/ style="([^"]*)"/, (_, s) => ` style="${fn(s)}"`);
}
/** UML realisation: dashed line, hollow triangle. */
export function realize(d, a, sa, b, sb, opts = {}) {
  const id = link(d, a, sa, b, sb, { dash: true, ...opts });
  restyle(d, id, (s) => s.replace('endFill=1', 'endFill=0'));
  return id;
}
/** Class box that warns when a member is wider than the box (monospace 12 px is about 7.3 px a character). */
import { cls as clsBase } from '../../../../paddock/diagrams/tools/uml.mjs';
export function klass(d, id, x, y, w, spec) {
  const max = Math.floor((w - 24) / 7.3);
  for (const m of [...(spec.attrs ?? []), ...(spec.ops ?? [])]) if (m.length > max) console.warn(`${id}: "${m}" is ${m.length} chars, box fits ${max}`);
  return clsBase(d, id, x, y, w, spec);
}
export const comp = { start: 'diamondThin', arrow: 'none' };

/**
 * Sequence diagram from a script. parts: [id, name, tech, role]; items: ['m', from, to, text, reply?] ['s', part, text]
 * ['o', kind, cond] ['e', cond] ['c'] ['n', part, text, width].
 */
export function seq(file, { title, kind = 'UML · Sequence diagram', desc, parts, script, pw = 168, gap = 40, legendRoles }) {
  const w = 96 + parts.length * pw + (parts.length - 1) * gap;
  const d = P({ title, kind, desc, w });
  const s = sequence(d, parts.map(([id, name, tech, role]) => ({ id, name, tech, role })), { pw, gap });
  let msgs = 0;
  for (const [k, ...a] of script) {
    if (k === 'm') { s.msg(a[0], a[1], a[2], { reply: !!a[3] }); msgs++; }
    else if (k === 's') s.self(a[0], a[1]);
    else if (k === 'o') s.open(a[0], a[1]);
    else if (k === 'e') s.else(a[0]);
    else if (k === 'c') s.close();
    else if (k === 'n') s.note(a[0], a[1], a[2]);
  }
  const y = s.done();
  if (msgs > 25) console.warn(`${file}: ${msgs} messages (limit 25)`);
  if (y + 120 > 1800) console.warn(`${file}: height ${y + 120} over 1800`);
  const names = { ui: 'Angular UI', accent: 'Debrief', core: 'Rust engine (pure)', process: 'Process · OS', external: 'External', plugin: 'Plugin' };
  d.legend([...[...new Set(parts.map((p) => p[3]))].map((k) => ({ kind: k, label: names[k] })), { kind: 'line', label: 'Call' }, { kind: 'dashline', label: 'Reply' }]);
  save(file, d, y);
}

/** A small loop above (or below) a node: from its edge at t1 out 28 px and back at t2. */
export function selfLoop(d, id, label, { side = 'T', t = [0.3, 0.7], dash = false } = {}) {
  const a = d.port(id, side, t[0]), b = d.port(id, side, t[1]);
  const off = side === 'T' ? -28 : 28;
  d.edge(id, id, a, b, { via: [[a[0], a[1] + off], [b[0], b[1] + off]], dash });
  d.lbl(label, (a[0] + b[0]) / 2, a[1] + off, side === 'T' ? 'a' : 'b');
}
/** An edge between two sides with its own pinned fractions and optional waypoints (for bends and return paths). */
export function route(d, a, sa, ta, b, sb, tb, via = [], opts = {}) {
  return d.edge(a, b, d.port(a, sa, ta), d.port(b, sb, tb), { via, ...opts });
}
