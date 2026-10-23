/**
 * Plausible plugin annotations for the browser demo, shaped like the Windows plugin's annotators
 * (crates/debrief-plugin-windows/src/annotators.rs): instance uptime spans, machine-wide downtime between
 * an instance that stopped and the next one on the same machine, and crash points. Not the reference
 * for behaviour: the engine decides what is annotated.
 */
import { AnnotationDto, AnnotationsRequest, Query, Severity } from '../api.types';
import { MockRecord } from './mock-data';

/** Owner of the demo annotations (the Windows plugin's id). */
export const MOCK_ANNOTATION_OWNER = 'net.decryptology.windows';
const MACHINE_FIELD = 'MachineName';

const query = (from: number, to: number, extra: Partial<Query> = {}): Query => ({
  from,
  to,
  levels: [],
  text: '',
  regex: false,
  caseSensitive: false,
  filters: [],
  groupBy: [],
  groups: [],
  markers: [],
  ...extra,
});

function note(
  annotator: string,
  kind: string,
  from: number,
  to: number,
  severity: Severity,
  label: string,
  rest: Pick<AnnotationDto, 'instance' | 'on' | 'evidenceQuery'>,
): AnnotationDto {
  return { annotator, owner: MOCK_ANNOTATION_OWNER, kind: `${MOCK_ANNOTATION_OWNER}/${kind}`, from, to, severity, label, ...rest };
}

interface Span {
  key: string;
  machine: string | null;
  first: number;
  last: number;
}

/**
 * The annotations over `records` overlapping `req`, by start time (like the engine: an instance filter keeps
 * the annotations for that instance plus those for every instance).
 */
export function mockAnnotations(records: readonly MockRecord[], keyOf: (r: MockRecord) => string, req: AnnotationsRequest = {}): AnnotationDto[] {
  const spans = new Map<string, Span>();
  const out: AnnotationDto[] = [];
  for (const r of records) {
    if (r.ts === null) continue;
    const key = keyOf(r);
    const s = spans.get(key);
    if (s) {
      s.first = Math.min(s.first, r.ts);
      s.last = Math.max(s.last, r.ts);
    } else {
      const m = r.fields[MACHINE_FIELD];
      spans.set(key, { key, machine: typeof m === 'string' ? m : null, first: r.ts, last: r.ts });
    }
    if (r.level === 'Fatal') {
      out.push(
        note('windows.crashes', 'app-crash', r.ts, r.ts, 'critical', `Application crashed: ${r.message.slice(0, 120)}`, {
          instance: key,
          on: null,
          evidenceQuery: query(r.ts - 60_000, r.ts + 1_000, { groups: [key], levels: ['Error', 'Fatal'] }),
        }),
      );
    }
  }
  const all = [...spans.values()];
  const end = Math.max(...all.map((s) => s.last));
  for (const s of all) {
    out.push(
      note('windows.services', 'service-running', s.first, s.last, s.last < end ? 'warning' : 'info', `${s.key} running`, {
        instance: s.key,
        on: null,
        evidenceQuery: query(s.first, s.last, { groups: [s.key] }),
      }),
    );
    // Downtime: this instance stopped and the next one on the same machine started later.
    const next = s.machine
      ? all.filter((o) => o !== s && o.machine === s.machine && o.first > s.last).sort((a, b) => a.first - b.first)[0]
      : undefined;
    if (next && s.machine) {
      out.push(
        note('windows.reboots', 'downtime', s.last, next.first, 'critical', `Unexpected shutdown — ${s.machine} down ${Math.round((next.first - s.last) / 1000)} s`, {
          instance: null,
          on: { field: MACHINE_FIELD, value: s.machine },
          evidenceQuery: query(s.last - 60_000, next.first + 60_000, { filters: [{ field: MACHINE_FIELD, op: 'eq', value: s.machine }] }),
        }),
      );
    }
  }
  return out
    .filter((a) => (req.from == null || a.to >= req.from) && (req.to == null || a.from <= req.to))
    .filter((a) => req.instance == null || a.instance === null || a.instance === req.instance)
    .sort((a, b) => a.from - b.from || a.kind.localeCompare(b.kind));
}
