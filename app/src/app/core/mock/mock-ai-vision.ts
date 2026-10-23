/**
 * Plausible image data for the scripted AI mock (ADR 0025): open images as the engine returns them (a small
 * generated clean copy, never the input decoded), and "find related logs" outcomes over the demo workspace.
 * Never the reference for behaviour: `crates/debrief-ai/src/vision` and `crates/debrief-core/src/related` are.
 */
import { AiImage, ImageSourceFormat, Narrative, RelatedEvent, RelatedLink, RelatedNote, RelatedOutcome, RelatedReason, ScreenFacts } from '../../ai/ai.types';
import { generateDemo, MockRecord } from './mock-data';

/** The base64 prefixes of the formats the engine reads (its signature check, mimicked without decoding). */
const SIGNATURES: [string, ImageSourceFormat][] = [
  ['iVBORw0KGgo', 'png'],
  ['/9j/', 'jpeg'],
  ['R0lGOD', 'gif'],
  ['Qk', 'bmp'],
  ['UklGR', 'webp'],
];

/** The format named by the first bytes of base64 image data, or null when it is none the engine reads. */
export function mockFormatOf(base64: string): ImageSourceFormat | null {
  return SIGNATURES.find(([prefix]) => base64.startsWith(prefix))?.[1] ?? null;
}

/** The format named by a file's extension, or null. */
export function mockFormatOfPath(path: string): ImageSourceFormat | null {
  const ext = /\.([a-z0-9]+)$/i.exec(path)?.[1]?.toLowerCase();
  if (ext === 'jpg' || ext === 'jpeg') return 'jpeg';
  return ext === 'png' || ext === 'bmp' || ext === 'gif' || ext === 'webp' ? ext : null;
}

/** A tiny "error dialog" screenshot as an SVG data URL: what the mock shows as the clean copy. */
export function mockCleanCopy(title: string): string {
  const safe = title.replace(/[^A-Za-z0-9 ._-]/g, '').slice(0, 40) || 'image';
  const svg =
    '<svg xmlns="http://www.w3.org/2000/svg" width="683" height="384" viewBox="0 0 683 384">' +
    '<rect width="683" height="384" fill="#1e2430"/>' +
    '<rect x="140" y="90" width="400" height="200" rx="6" fill="#f4f4f4" stroke="#888"/>' +
    '<rect x="140" y="90" width="400" height="28" rx="6" fill="#d9dde3"/>' +
    `<text x="152" y="109" font-family="sans-serif" font-size="13" fill="#222">OrderService - ${safe}</text>` +
    '<circle cx="182" cy="160" r="16" fill="#c62828"/>' +
    '<text x="210" y="157" font-family="sans-serif" font-size="13" fill="#222">System.OutOfMemoryException:</text>' +
    '<text x="210" y="176" font-family="sans-serif" font-size="13" fill="#222">Insufficient memory to continue.</text>' +
    '<text x="210" y="205" font-family="sans-serif" font-size="12" fill="#555">14.10.2026 10:41:26  WEB02</text>' +
    '<rect x="440" y="246" width="84" height="28" rx="3" fill="#e0e0e0" stroke="#999"/>' +
    '<text x="470" y="265" font-family="sans-serif" font-size="13" fill="#222">OK</text>' +
    '</svg>';
  return `data:image/svg+xml;base64,${btoa(svg)}`;
}

export function mockImage(id: number, name: string, path: string | null, format: ImageSourceFormat): AiImage {
  return {
    id,
    name,
    path,
    width: 1366,
    height: 768,
    originalWidth: 2732,
    originalHeight: 1536,
    sourceFormat: format,
    dataUrl: mockCleanCopy(name),
    // Photos carry an EXIF capture time; screenshots usually do not.
    capturedAt: format === 'jpeg' ? '2026-10-14 10:41:26' : null,
  };
}

/** Which mock scenario an image name asks for: `clock`/`skew` (the clocks disagree), `console`/`notime` (no time). */
export type MockRelatedScenario = 'normal' | 'clock' | 'noTime';

export function mockScenario(name: string): MockRelatedScenario {
  const n = name.toLowerCase();
  if (/clock|skew/.test(n)) return 'clock';
  if (/console|notime/.test(n)) return 'noTime';
  return 'normal';
}

const SOURCE_NAMES = ['orders-20261014.clef', 'orders-20261014.clef', 'worker-20261014.log', 'gateway-20261014.json', 'inventory-sync.csv', 'registry-export.reg'];

let sorted: MockRecord[] | null = null;

/** The demo records in the mock engine's id order (time order; ids are positions, like `demoRecordIds`). */
function demoRecords(): MockRecord[] {
  sorted ??= [...generateDemo().records].sort((a, b) => (a.ts ?? Number.MAX_SAFE_INTEGER) - (b.ts ?? Number.MAX_SAFE_INTEGER) || a.source - b.source || a.line - b.line);
  return sorted;
}

function eventOf(r: MockRecord, id: number): RelatedEvent {
  const machine = typeof r.fields['MachineName'] === 'string' ? r.fields['MachineName'] : '';
  const pid = r.fields['ProcessId'];
  return {
    id,
    ts: r.ts,
    level: r.level,
    message: r.message,
    exception: r.exception,
    source: r.source,
    sourceName: SOURCE_NAMES[r.source] ?? `source ${r.source}`,
    group: machine ? `${machine} · ${String(pid ?? '')}` : '',
    fields: { Application: r.fields['Application'], MachineName: machine },
  };
}

const EXCEPTION = /([A-Z]\w*(?:\.\w+)*Exception)/;

function facts(scenario: MockRelatedScenario): ScreenFacts {
  return {
    screenKind: scenario === 'noTime' ? 'console' : 'error_dialog',
    windowTitle: 'OrderService',
    application: 'OrderService',
    timestamps: scenario === 'noTime' ? [] : [{ text: '14.10.2026 10:41:26', hasDate: true }],
    errorTexts: ['Insufficient memory to continue.'],
    errorCodes: [],
    exceptionNames: ['System.OutOfMemoryException'],
    machineNames: ['WEB02'],
    userNames: [],
    serverNames: [],
    ids: [],
    versions: [],
    summary: 'OrderService on WEB02 shows an out-of-memory error dialog.',
  };
}

/** "Find related logs": the crash of WEB02 and the errors around it, ranked like the engine would. */
export function mockRelatedOutcome(name: string): RelatedOutcome {
  const scenario = mockScenario(name);
  const records = demoRecords();
  const fatalAt = Math.max(0, records.findIndex((r) => r.level === 'Fatal'));
  const fatal = records[fatalAt];
  // The clock scenario: the screenshot's machine runs 3 min ahead, so the strong match is outside the window.
  const imageAt = (fatal?.ts ?? 0) + (scenario === 'clock' ? 180_000 : 1_400);
  const near = records
    .map((r, id) => ({ r, id }))
    .filter(({ r, id }) => id !== fatalAt && (r.level === 'Error' || r.level === 'Fatal') && r.ts !== null && Math.abs(r.ts - imageAt) <= 120_000)
    .slice(-3);
  const linkOf = (r: MockRecord, id: number, score: number, strong: boolean): RelatedLink => {
    const reasons: RelatedReason[] = [];
    if (scenario !== 'noTime' && r.ts !== null && Math.abs(r.ts - imageAt) <= 120_000) reasons.push({ kind: 'time', deltaMs: r.ts - imageAt });
    if (r.fields['Application'] === 'OrderService') reasons.push({ kind: 'title', text: 'OrderService', target: { kind: 'field', field: 'Application', value: 'OrderService' } });
    const ex = EXCEPTION.exec(r.exception ?? r.message)?.[1];
    if (strong && ex) reasons.push({ kind: 'text', clue: ex, clueKind: 'exceptionName', partial: false });
    else reasons.push({ kind: 'text', clue: 'Insufficient memory to continue.', clueKind: 'errorText', partial: true });
    return { event: eventOf(r, id), score, reasons };
  };
  const links: RelatedLink[] = fatal ? [linkOf(fatal, fatalAt, 9.5, true)] : [];
  near.forEach(({ r, id }, i) => links.push(linkOf(r, id, 5 - i, false)));
  const notes: RelatedNote[] = [];
  if (scenario === 'noTime') notes.push({ kind: 'noTime' });
  if (scenario === 'clock' && fatal) notes.push({ kind: 'clockDisagreement', deltaMs: (fatal.ts ?? 0) - imageAt, sourceId: fatal.source, sourceName: SOURCE_NAMES[fatal.source] ?? '' });
  const explanation: Narrative = {
    text: fatal
      ? `The dialog shows OrderService on WEB02 running out of memory. It matches the crash at event ${fatalAt}${near[0] ? `, after the errors at event ${near[0].id}` : ''}.`
      : 'Nothing in the loaded logs matches this image.',
    citations: [...(fatal ? [{ kind: 'record' as const, id: fatalAt, verified: true }] : []), ...(near[0] ? [{ kind: 'record' as const, id: near[0].id, verified: true }] : [])],
    evidence: { recordIds: links.map((l) => l.event.id), queries: [] },
    usage: { inputTokens: 1800, outputTokens: 70 },
  };
  return {
    facts: facts(scenario),
    result: {
      time: scenario === 'noTime' ? null : { atMs: imageAt, precisionMs: 1000, source: 'screen', text: '14.10.2026 10:41:26' },
      links,
      notes,
      examined: 412,
    },
    explanation,
  };
}

/** "Explain with AI" on an image. */
export function mockExplainImage(name: string, question: string | null): Narrative {
  const { result } = mockRelatedOutcome(name);
  const top = result.links[0]?.event.id;
  const ask = question?.trim() ? ` You asked: "${question.trim().slice(0, 80)}".` : '';
  return {
    text: `This is an **error dialog** of OrderService on WEB02: \`System.OutOfMemoryException\`, "Insufficient memory to continue".${ask}${top !== undefined ? `\n\n- The crash at event ${top} is the likely moment` : ''}\n- Next step: use **Find related logs** to see the events around it`,
    citations: top !== undefined ? [{ kind: 'record', id: top, verified: true }] : [],
    evidence: { recordIds: top !== undefined ? [top] : [], queries: [] },
    usage: { inputTokens: 1300, outputTokens: 60 },
  };
}

/** The chat's related-logs intent, roughly as `debrief_ai::vision::intent` recognises it. */
export function mockAsksForRelated(text: string): boolean {
  const t = text.trim().toLowerCase();
  if (t === '/related' || t.startsWith('/related ')) return true;
  return /(find|show|which|related|match|around)/.test(t) && /(log|event|entr|line|trace)/.test(t);
}
