import { FieldFilter, Level, LEVELS, LensCfg, LensHighlight, Query } from '../core/api.types';

/** Version of the lens and filter-set state saved in projects and sessions. */
export const LENS_STATE_VERSION = 1;

/** Most saved lenses and filter sets a project keeps (saved state is untrusted input). */
export const MAX_SAVED = 64;

const FILTER_OPS = new Set<string>(['eq', 'ne', 'contains', 'notContains', 'matches', 'exists', 'missing', 'gt', 'lt']);
const HIGHLIGHT_OPS = new Set<string>(['eq', 'ne', 'contains', 'notContains', 'exists', 'missing', 'gt', 'lt']);
const COLOR = /^#[0-9a-fA-F]{6}$/;

/** The part of the query a lens sets. Time range, instance selection and sources are not part of a view preset. */
export type LensQuery = Pick<Query, 'levels' | 'text' | 'regex' | 'ql' | 'markers' | 'filters'>;

/** A lens as the UI works with it: built in (from the profile) or saved by the user. */
export interface Lens {
  id: string;
  name: string;
  description: string;
  query: LensQuery;
  /** Extra grid columns; empty = leave the columns as they are. */
  columns: string[];
  /** Row colours added when the lens is applied. */
  highlights: LensHighlight[];
  /** Profile chart shown first; null = none. */
  chart: string | null;
}

/** A named group of filters that can be switched on and off without deleting it. */
export interface FilterSet {
  id: string;
  name: string;
  filters: FieldFilter[];
  enabled: boolean;
}

/** A profile lens as a UI lens (the engine validated it). */
export function lensFromProfile(cfg: LensCfg): Lens {
  return {
    id: cfg.id,
    name: cfg.title,
    description: cfg.description ?? '',
    query: { levels: cfg.levels ?? [], text: cfg.text ?? '', regex: cfg.regex ?? false, ql: cfg.ql ?? '', markers: cfg.markers ?? [], filters: cfg.filter ?? [] },
    columns: cfg.columns ?? [],
    highlights: cfg.highlight ?? [],
    chart: cfg.chart ?? null,
  };
}

/** The lens-relevant part of a query. */
export function lensQueryOf(q: Query): LensQuery {
  return { levels: [...q.levels], text: q.text, regex: q.regex, ql: q.ql ?? '', markers: [...q.markers], filters: q.filters.map((f) => ({ ...f })) };
}

const isRecord = (v: unknown): v is Record<string, unknown> => v !== null && typeof v === 'object' && !Array.isArray(v);
const strings = (v: unknown): string[] => (Array.isArray(v) ? v.filter((s): s is string => typeof s === 'string') : []);

function parseFilters(v: unknown): FieldFilter[] {
  if (!Array.isArray(v)) return [];
  return v.flatMap((f): FieldFilter[] => {
    if (!isRecord(f) || typeof f['field'] !== 'string' || typeof f['op'] !== 'string' || !FILTER_OPS.has(f['op'])) return [];
    const filter: FieldFilter = { field: f['field'], op: f['op'] as FieldFilter['op'] };
    if (typeof f['value'] === 'string') filter.value = f['value'];
    return [filter];
  });
}

function parseHighlights(v: unknown): LensHighlight[] {
  if (!Array.isArray(v)) return [];
  return v.flatMap((h): LensHighlight[] => {
    if (!isRecord(h) || typeof h['field'] !== 'string' || typeof h['op'] !== 'string' || !HIGHLIGHT_OPS.has(h['op'])) return [];
    if (typeof h['color'] !== 'string' || !COLOR.test(h['color'])) return [];
    return [{ field: h['field'], op: h['op'] as LensHighlight['op'], value: typeof h['value'] === 'string' ? h['value'] : '', color: h['color'] }];
  });
}

/** The items of a saved `{ version, items }` value; a missing, malformed or newer-version value gives none. */
function itemsOf(value: unknown): unknown[] {
  if (!isRecord(value) || value['version'] !== LENS_STATE_VERSION || !Array.isArray(value['items'])) return [];
  return value['items'].slice(0, MAX_SAVED);
}

/** Saved lenses from untrusted state; entries that are not lenses are dropped, the rest is kept. */
export function parseSavedLenses(value: unknown): Lens[] {
  return itemsOf(value).flatMap((raw): Lens[] => {
    if (!isRecord(raw) || typeof raw['id'] !== 'string' || typeof raw['name'] !== 'string' || !isRecord(raw['query'])) return [];
    const q = raw['query'];
    const levels = strings(q['levels']).filter((l): l is Level => (LEVELS as string[]).includes(l));
    return [
      {
        id: raw['id'],
        name: raw['name'],
        description: typeof raw['description'] === 'string' ? raw['description'] : '',
        query: { levels, text: typeof q['text'] === 'string' ? q['text'] : '', regex: q['regex'] === true, ql: typeof q['ql'] === 'string' ? q['ql'] : '', markers: strings(q['markers']), filters: parseFilters(q['filters']) },
        columns: strings(raw['columns']),
        highlights: parseHighlights(raw['highlights']),
        chart: typeof raw['chart'] === 'string' ? raw['chart'] : null,
      },
    ];
  });
}

/** Saved filter sets from untrusted state (same rules as lenses). */
export function parseFilterSets(value: unknown): FilterSet[] {
  return itemsOf(value).flatMap((raw): FilterSet[] => {
    if (!isRecord(raw) || typeof raw['id'] !== 'string' || typeof raw['name'] !== 'string') return [];
    return [{ id: raw['id'], name: raw['name'], filters: parseFilters(raw['filters']), enabled: raw['enabled'] !== false }];
  });
}

/** The saved form of a list: versioned so a later format can be told apart. */
export function versioned<T>(items: readonly T[]): { version: number; items: readonly T[] } {
  return { version: LENS_STATE_VERSION, items };
}
