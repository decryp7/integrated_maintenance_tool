import { DashboardCfg, TILE_KINDS, TileCfg, TileKind } from '../../core/api.types';

/**
 * The profile's `[[dashboard]]`s as the engine reports them in the summary (defaults filled in), and the
 * engine's validation of them, approximated for browser development. The engine is the reference.
 */

const num = (v: unknown, d: number) => (typeof v === 'number' && Number.isFinite(v) ? v : d);
const str = (v: unknown) => (typeof v === 'string' ? v : undefined);

function tileOf(raw: Record<string, unknown>): TileCfg {
  const t: TileCfg = {
    id: String(raw['id'] ?? ''),
    title: str(raw['title']) ?? '',
    type: (str(raw['type']) ?? 'stat') as TileKind,
    x: num(raw['x'], 0),
    y: num(raw['y'], 0),
    w: num(raw['w'], 4),
    h: num(raw['h'], 3),
  };
  for (const k of ['query', 'chart', 'value', 'style', 'field', 'unit', 'window'] as const) {
    const v = str(raw[k]);
    if (v !== undefined) (t as unknown as Record<string, unknown>)[k] = v;
  }
  if (Array.isArray(raw['by'])) t.by = (raw['by'] as unknown[]).map(String);
  if (typeof raw['limit'] === 'number') t.limit = raw['limit'];
  return t;
}

export function mockDashboards(profile: Record<string, unknown>): DashboardCfg[] {
  const list = Array.isArray(profile['dashboard']) ? (profile['dashboard'] as Record<string, unknown>[]) : [];
  return list.map((d) => ({
    id: String(d['id'] ?? ''),
    title: String(d['title'] ?? ''),
    columns: num(d['columns'], 12),
    tile: Array.isArray(d['tile']) ? (d['tile'] as Record<string, unknown>[]).map(tileOf) : [],
  }));
}

/** The first problem with a tile, as the engine words it; null when it is valid. */
export function mockTileError(t: TileCfg, columns = 24): string | null {
  if (!t.id.trim()) return 'a tile needs an id';
  if (!TILE_KINDS.includes(t.type)) return `tile "${t.id}": unknown type "${t.type}"`;
  if (t.w < 1 || t.h < 1 || t.h > 48) return `tile "${t.id}": w and h must be at least 1 (h at most 48)`;
  if (t.x + t.w > columns) return `tile "${t.id}": x + w must be at most the dashboard's ${columns} columns`;
  if (t.window !== undefined && !/^\d+(\.\d+)?\s*(ms|s|m|h|d)?$/.test(t.window.trim())) return `tile "${t.id}": window: "${t.window}" is not a duration`;
  const q = t.query?.trim() ?? '';
  if ((t.type === 'stat' || t.type === 'table') && !q) return `tile "${t.id}": a ${t.type} tile needs a query`;
  if (t.type === 'stat' && !/\|\s*stats\b/i.test(q)) return `tile "${t.id}": a stat tile needs a \`| stats ...\` query`;
  if (t.type === 'chart' && !t.chart && !t.value) return `tile "${t.id}": a chart tile needs \`chart\` (a [[chart]] id) or \`value\``;
  if (t.type === 'top' && !t.field?.trim()) return `tile "${t.id}": a top tile needs a \`field\``;
  return null;
}

/** Validates every dashboard like the engine does when a profile is saved. */
export function mockDashboardsError(dashboards: DashboardCfg[]): string | null {
  const ids = new Set<string>();
  for (const d of dashboards) {
    if (!d.id.trim()) return 'a [[dashboard]] needs an id';
    if (ids.has(d.id)) return `dashboard id "${d.id}" is used twice`;
    ids.add(d.id);
    if (d.columns < 1 || d.columns > 24) return `dashboard "${d.id}": columns must be 1 to 24`;
    const tiles = new Set<string>();
    for (const t of d.tile) {
      const e = mockTileError(t, d.columns);
      if (e) return `dashboard "${d.id}": ${e}`;
      if (tiles.has(t.id)) return `dashboard "${d.id}": tile id "${t.id}" is used twice`;
      tiles.add(t.id);
    }
  }
  return null;
}
