import { P, save, link, restyle, comp, Y0 } from './common.mjs';
import { ld } from '../../../../paddock/diagrams/tools/lib.mjs';

const esc = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
function ellipse(d, id, x, y, w, h, text) {
  text = text.replace(/`/g, '');
  d.rect[id] = [x, y, w, h];
  const a = d.t.accent;
  d.raw(`<mxCell id="${id}" value="${esc(`<span style="font-size:12px">${esc(text)}</span>`)}" style="${d.base(`ellipse;strokeWidth=1.5;strokeColor=${ld(a.s)};fillColor=${ld(a.f)};align=center;verticalAlign=middle;spacingLeft=24;spacingRight=24;`)}" vertex="1" parent="1"><mxGeometry x="${x}" y="${y}" width="${w}" height="${h}" as="geometry"/></mxCell>`);
}
function actor(d, id, x, y, name) {
  d.rect[id] = [x, y, 40, 64];
  d.raw(`<mxCell id="${id}" value="${esc(`<span style="font-size:12px;font-weight:600">${esc(name)}</span>`)}" style="${d.base(`shape=umlActor;verticalLabelPosition=bottom;verticalAlign=top;strokeWidth=1.5;strokeColor=${ld(d.t.process.s)};fillColor=none;whiteSpace=nowrap;`)}" vertex="1" parent="1"><mxGeometry x="${x}" y="${y}" width="40" height="64" as="geometry"/></mxCell>`);
}
/**
 * Use case diagram: one column of use cases in a boundary, actors on the left (each fans out to its own rows),
 * «include» / «extend» satellites in a second column.
 * actors: [{ name, rows: [first, last] | null }]; sats: [{ text, row, from, kind }]; general: [[specialisedIndex, generalIndex]].
 */
export function useCases(file, { title, desc, bound, rows, actors, sats = [], general = [], ext = null }) {
  const UW = 336, UH = 56, PITCH = 80, bx = 248;
  const sw = sats.length ? 112 + 296 : 0;
  const bw = 24 + UW + sw + 24, nrows = Math.max(rows.length, ...sats.map((s) => s.row + 1));
  const bh = nrows * PITCH + 40;
  const d = P({ title, kind: 'UML · Use case diagram', desc, w: bx + bw + (ext ? 120 + 96 : 0) + 48 });
  d.group('b', bx, Y0, bw, bh, bound);
  const cy = (i) => Y0 + 40 + i * PITCH + UH / 2;
  rows.forEach((t, i) => ellipse(d, `u${i}`, bx + 24, Y0 + 40 + i * PITCH, UW, UH, t));
  const trunk = bx - 40;
  const pos = [];
  actors.forEach((a, k) => {
    let mid;
    if (a.rows) mid = Math.round((cy(a.rows[0]) + cy(a.rows[1])) / 2);
    else mid = pos[general.find(([s]) => s === k)?.[1]] + 168;
    pos.push(mid);
    actor(d, `a${k}`, 72, mid - 32, a.name);
    if (a.rows) for (let i = a.rows[0]; i <= a.rows[1]; i++) {
      d.edge(`a${k}`, `u${i}`, [112, mid], [bx + 24, cy(i)], { arrow: 'none', via: mid === Math.round(cy(i)) ? [] : [[trunk, mid], [trunk, Math.round(cy(i))]] });
    }
  });
  for (const [s, g] of general) {
    const id = d.edge(`a${s}`, `a${g}`, [92, pos[s] - 32], [92, pos[g] + 32 + 28], { arrow: 'block' });
    restyle(d, id, (st) => st.replace('endFill=1', 'endFill=0'));
    d.lbl('Generalisation', 92, (pos[s] - 32 + pos[g] + 60) / 2, 'r');
  }
  const sx = bx + 24 + UW + 112, gapX = bx + 24 + UW + 56;
  sats.forEach((s, i) => {
    ellipse(d, `s${i}`, sx, Y0 + 40 + s.row * PITCH, 296, UH, s.text);
    const froms = [].concat(s.from);
    froms.forEach((f, k) => {
      const a = [bx + 24 + UW, Math.round(cy(f))], b = [sx, Math.round(cy(s.row))];
      d.edge(`u${f}`, `s${i}`, a, b, { dash: true, arrow: 'open', via: a[1] === b[1] ? [] : [[gapX, a[1]], [gapX, b[1]]] });
      if (a[1] === b[1]) d.lbl(s.kind, (a[0] + b[0]) / 2, a[1], 'a');
    });
  });
  if (ext) {
    actor(d, 'x', bx + bw + 120, Math.round(cy(ext.row) - 32), ext.name);
    d.edge('x', `u${ext.row}`, [bx + bw + 120, Math.round(cy(ext.row))], [bx + bw - 24, Math.round(cy(ext.row))], { arrow: 'none' });
  }
  d.legend([{ kind: 'accent', label: 'Use case' }, { kind: 'line', label: 'Association' }, ...(sats.length ? [{ kind: 'dashline', label: '«include» / «extend»' }] : [])]);
  save(file, d, Y0 + bh + (actors.length > 1 ? 0 : 0));
}
