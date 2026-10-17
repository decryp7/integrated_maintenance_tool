import type { MockBackendOptions } from './mock-backend';

/** Commands that `mockFail` may name (the mock rejects their first N calls). */
export const FAILABLE = ['workspaceSummary', 'queryPage', 'queryTimeline', 'queryFacet', 'sourceContext', 'readFileHead', 'previewParse'] as const;
export type Failable = (typeof FAILABLE)[number];

function positiveInt(v: string | null): number | undefined {
  if (v === null || !/^\d{1,12}$/.test(v)) return undefined;
  const n = Number(v);
  return n > 0 ? n : undefined;
}

/**
 * Browser-demo switches read from the URL, for UI tests and manual checks (never used inside Tauri):
 *   ?mockTotal=50000000       synthetic result of that many events (lazily generated)
 *   ?mockFacet=10000          every field has that many distinct values
 *   ?mockFail=queryPage:2     the first N calls of a command reject (comma separated)
 *   ?mockCorrupt=markers      page rows arrive without `markers`, as a buggy backend would send them
 *   ?mockLiveMs=300           interval of the live tail events (default 1500)
 */
export function mockOptionsFromSearch(search: string): MockBackendOptions {
  const p = new URLSearchParams(search);
  const options: MockBackendOptions = {};
  const total = positiveInt(p.get('mockTotal'));
  if (total) options.syntheticTotal = total;
  const facet = positiveInt(p.get('mockFacet'));
  if (facet) options.syntheticFacetValues = facet;
  const liveMs = positiveInt(p.get('mockLiveMs'));
  if (liveMs) options.liveMs = liveMs;
  if (p.get('mockCorrupt') === 'markers') options.corruptMarkers = true;
  const fail: Partial<Record<Failable, number>> = {};
  for (const part of (p.get('mockFail') ?? '').split(',')) {
    const [name, count] = part.split(':');
    const n = positiveInt(count ?? '1') ?? 1;
    if ((FAILABLE as readonly string[]).includes(name)) fail[name as Failable] = n;
  }
  if (Object.keys(fail).length) options.fail = fail;
  return options;
}
