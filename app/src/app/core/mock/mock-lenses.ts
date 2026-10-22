import { LensCfg } from '../api.types';

/** The profile's `[[lens]]`s as the engine reports them in the summary (the engine is the reference; this serves browser development). */
export function mockLenses(profile: Record<string, unknown>): LensCfg[] {
  const list = Array.isArray(profile['lens']) ? (profile['lens'] as Record<string, unknown>[]) : [];
  return list.map((raw) => {
    const lens = { ...raw, id: String(raw['id'] ?? ''), title: String(raw['title'] ?? '') } as Record<string, unknown>;
    // TOML nests filters and highlights as `filter` / `highlight` tables, exactly the summary's keys.
    return lens as unknown as LensCfg;
  });
}
