import type { MockBackendOptions } from './mock-backend';

/** Commands that `mockFail` may name (the mock rejects their first N calls). */
export const FAILABLE = ['workspaceSummary', 'queryPage', 'queryTimeline', 'queryFacet', 'sourceContext', 'readFileHead', 'previewParse', 'annotations', 'queryRuns', 'previewSpans'] as const;
export type Failable = (typeof FAILABLE)[number];

function positiveInt(v: string | null): number | undefined {
  if (v === null || !/^\d{1,12}$/.test(v)) return undefined;
  const n = Number(v);
  return n > 0 ? n : undefined;
}

function nonNegativeInt(v: string | null): number | undefined {
  return v === '0' ? 0 : positiveInt(v);
}

/**
 * Browser-demo switches read from the URL, for UI tests and manual checks (never used inside Tauri):
 *   ?mockTotal=50000000       synthetic result of that many events (lazily generated)
 *   ?mockFacet=10000          every field has that many distinct values
 *   ?mockFail=queryPage:2     the first N calls of a command reject (comma separated)
 *   ?mockMemoryFeatures=1     also offer what only the old in-memory store did (clock skew, alert mutes, tail state, redacted view)
 *   ?mockCorrupt=markers      page rows arrive without `markers`, as a buggy backend would send them
 *   ?mockLiveMs=300           interval of the live tail events (default 1500; 250 with mockLive)
 *   ?mockLive=1               realistic live simulation: records across files, rotations, alerts (implies the switches below)
 *   ?mockLiveRate=50          records per second while live tail is on (default 20 with mockLive)
 *   ?mockLiveRotateMs=30000   rotate the tailed file this often (0 = never)
 *   ?mockLiveAlertMs=10000    raise a live alert this often (0 = never)
 *   ?mockSlowImport=600       opening is slow and cancellable: each path is a folder of files read this many ms apart
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
  const liveRate = positiveInt(p.get('mockLiveRate'));
  if (p.get('mockLive') === '1' || liveRate) {
    options.liveRate = liveRate ?? 20;
    options.exposeLiveCalls = true;
    const rotate = nonNegativeInt(p.get('mockLiveRotateMs'));
    if (rotate !== undefined) options.liveRotateMs = rotate;
    const alert = nonNegativeInt(p.get('mockLiveAlertMs'));
    if (alert !== undefined) options.liveAlertMs = alert;
  }
  if (p.get('mockCorrupt') === 'markers') options.corruptMarkers = true;
  const slow = positiveInt(p.get('mockSlowImport'));
  if (slow) options.slowImportMs = Math.min(slow, 10_000);
  const fail: Partial<Record<Failable, number>> = {};
  for (const part of (p.get('mockFail') ?? '').split(',')) {
    const [name, count] = part.split(':');
    const n = positiveInt(count ?? '1') ?? 1;
    if (name !== undefined && (FAILABLE as readonly string[]).includes(name)) fail[name as Failable] = n;
  }
  if (Object.keys(fail).length) options.fail = fail;
  if (p.get('mockMemoryFeatures') === '1') options.memoryStoreFeatures = true;
  return options;
}
