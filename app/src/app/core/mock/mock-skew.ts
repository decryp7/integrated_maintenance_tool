import { OffsetSuggestion, SetOffsetRequest, SuggestRequest, SuggestResult } from '../api.types';
import { MockRecord, MockSource } from './mock-data';

/**
 * Clock-skew correction for the mock backend: mimics `set_source_offset` / `suggest_offsets` of the
 * engine (crates/imt-core/src/skew) with plausible results. The engine is the reference.
 */

export const MOCK_MAX_OFFSET_MS = 7 * 24 * 3_600_000;
const DEFAULT_FIELDS = ['CorrelationId', 'TraceId', 'RequestId'];
const MIN_PAIRS = 3;

/** Messages like the engine's `SkewError`. */
export function checkOffset(ms: number): number {
  if (!Number.isFinite(ms) || Math.abs(ms) > MOCK_MAX_OFFSET_MS) throw new Error(`a clock offset of ${ms} ms is too large; use at most ±7 days`);
  return Math.round(ms);
}

/** The source and absolute offset a request asks for, resolved against the shown records (ids = positions). */
export function resolveOffset(req: SetOffsetRequest, shown: readonly MockRecord[], sources: readonly MockSource[]): { source: number; offsetMs: number } {
  if (req.kind === 'manual') {
    if (!sources.some((s) => s.id === req.source)) throw new Error(`source ${req.source} is not open any more; reload the sources and try again`);
    return { source: req.source, offsetMs: checkOffset(req.offsetMs) };
  }
  const timed = (id: number) => {
    const r = shown[id];
    if (!r) throw new Error(`event ${id} no longer exists; select the events again`);
    if (r.ts === null) throw new Error(`event ${id} has no timestamp, so it cannot be used to align clocks`);
    return r as MockRecord & { ts: number };
  };
  const ref = timed(req.reference);
  const target = timed(req.target);
  if (ref.source === target.source) throw new Error(`both events come from the same source (${ref.source}); pick the matching event in another source`);
  const current = sources.find((s) => s.id === target.source)?.offsetMs ?? 0;
  return { source: target.source, offsetMs: checkOffset(ref.ts - (target.ts - current)) };
}

/** Moves the raw records of `source` by the difference to the new offset; returns the previous offset. */
export function shiftSource(raw: MockRecord[], sources: MockSource[], source: number, offsetMs: number): number {
  const src = sources.find((s) => s.id === source);
  const previous = src?.offsetMs ?? 0;
  const delta = offsetMs - previous;
  if (src) src.offsetMs = offsetMs;
  if (delta !== 0) for (const r of raw) if (r.source === source && r.ts !== null) r.ts += delta;
  return previous;
}

/** Identity of a record across renumbering. */
export function recordKey(r: Pick<MockRecord, 'source' | 'line'>): string {
  return `${r.source}:${r.line}`;
}

/** First position whose record differs between two orderings (null when none). */
export function firstChanged(before: readonly MockRecord[], after: readonly MockRecord[]): number | null {
  const n = Math.max(before.length, after.length);
  for (let i = 0; i < n; i++) {
    const a = before[i];
    const b = after[i];
    if (!a || !b || a.source !== b.source || a.line !== b.line || a.ts !== b.ts) return i;
  }
  return null;
}

function median(values: number[]): number {
  const v = [...values].sort((a, b) => a - b);
  return v[Math.floor((v.length - 1) / 2)] ?? 0;
}

/** Median of paired correlation-id deltas against a reference source, like the engine. */
export function mockSuggest(records: readonly MockRecord[], sources: readonly MockSource[], req: SuggestRequest = {}): SuggestResult {
  const fields = req.fields?.length ? req.fields : DEFAULT_FIELDS;
  const bound = req.maxDeltaMs ?? 3_600_000;
  const tolerance = req.toleranceMs ?? 1000;
  const offset = (s: number) => sources.find((x) => x.id === s)?.offsetMs ?? 0;
  const byId = new Map<string, Map<number, number>>();
  for (const r of records) {
    if (r.ts === null) continue;
    const id = fields.map((f) => r.fields[f]).find((v): v is string => typeof v === 'string' && v !== '');
    if (!id) continue;
    const original = r.ts - offset(r.source);
    const m = byId.get(id) ?? new Map<number, number>();
    m.set(r.source, Math.min(m.get(r.source) ?? Infinity, original));
    byId.set(id, m);
  }
  const links = new Map<number, number>();
  for (const m of byId.values()) if (m.size > 1) for (const s of m.keys()) links.set(s, (links.get(s) ?? 0) + m.size - 1);
  const reference = req.reference ?? [...links.entries()].sort((a, b) => b[1] - a[1] || a[0] - b[0])[0]?.[0] ?? null;
  if (reference === null) return { reference: null, fields, suggestions: [], insufficient: [] };
  const deltas = new Map<number, number[]>();
  for (const m of byId.values()) {
    const r = m.get(reference);
    if (r === undefined) continue;
    for (const [s, t] of m) {
      if (s === reference) continue;
      const d = r + offset(reference) - t;
      if (Math.abs(d) <= bound) deltas.set(s, [...(deltas.get(s) ?? []), d]);
    }
  }
  const suggestions: OffsetSuggestion[] = [];
  const insufficient: number[] = [];
  for (const s of [...links.keys()].filter((x) => x !== reference).sort((a, b) => a - b)) {
    const d = deltas.get(s) ?? [];
    const med = d.length ? median(d) : 0;
    const agreeing = d.filter((x) => Math.abs(x - med) <= tolerance).length;
    if (d.length < MIN_PAIRS || agreeing < MIN_PAIRS) {
      insufficient.push(s);
      continue;
    }
    const confidence = Math.round((agreeing / d.length) * Math.min(1, d.length / 10) * 1000) / 1000;
    suggestions.push({ source: s, reference, currentMs: offset(s), suggestedMs: med, pairs: d.length, agreeing, spreadMs: median(d.map((x) => Math.abs(x - med))), confidence });
  }
  return { reference, fields, suggestions, insufficient };
}
