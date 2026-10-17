import { Facet, FacetRequest, LEVELS, Page, PageRequest, RecordView } from '../api.types';
import { DEMO_START } from './mock-data';

/**
 * Lazily generated events for scale tests: `total` rows exist only by index, nothing is stored.
 * Row `i` has record id `i`, so ascending position = id and descending position = total - 1 - id.
 * Queries are ignored (every row matches); this exists to exercise the grid's scrolling, not the query engine.
 */
export function syntheticRow(i: number): RecordView {
  const level = i % 97 === 0 ? 'Error' : i % 7 === 0 ? 'Warning' : 'Information';
  return {
    id: i,
    ts: DEMO_START + i * 1000,
    level,
    message: `Synthetic event ${i}`,
    template: null,
    exception: null,
    fields: { Index: i, Bucket: LEVELS[i % LEVELS.length] },
    source: 0,
    sourceName: 'synthetic.log',
    line: i + 1,
    group: 'synthetic',
    markers: [],
  };
}

export function syntheticPage(req: PageRequest, total: number): Page {
  const toIndex = (pos: number) => (req.descending ? total - 1 - pos : pos);
  let offset = Math.max(0, Math.min(req.offset, Math.max(0, total - 1)));
  let anchorIndex: number | null = null;
  if (req.anchor !== null && req.anchor !== undefined && Number.isInteger(req.anchor) && req.anchor >= 0 && req.anchor < total) {
    anchorIndex = req.descending ? total - 1 - req.anchor : req.anchor;
    offset = Math.max(0, anchorIndex - Math.floor(req.limit / 2));
  }
  const end = Math.min(total, offset + Math.max(0, req.limit));
  const rows: RecordView[] = [];
  for (let p = offset; p < end; p++) rows.push(syntheticRow(toIndex(p)));
  return { total, offset, rows, anchorIndex };
}

/** The engine clamps facet limits to 1000 values; `distinct` is reported in full. */
export const FACET_LIMIT_MAX = 1000;

export function syntheticFacet(req: FacetRequest, distinct: number, total = 1_000_000): Facet {
  const shown = Math.min(distinct, Math.max(1, Math.min(req.limit, FACET_LIMIT_MAX)));
  const values = Array.from({ length: shown }, (_, k) => ({ value: `${req.field}-value-${String(k).padStart(5, '0')}`, count: Math.max(1, distinct - k) }));
  return { field: req.field, values, distinct, total };
}
