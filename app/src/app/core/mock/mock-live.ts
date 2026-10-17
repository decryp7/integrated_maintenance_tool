import { AlertTarget, LiveAlert, LiveOptions, MAX_DETECT_EVERY_MS, MAX_MUTE_MS, Query, TailFileState, TailFileStatus, TailReport, TailStatus, WatchMode } from '../api.types';
import { MockRecord, MockSource } from './mock-data';

/**
 * Live tail simulation for the browser demo (mock only; the real behaviour is the engine's
 * `workspace::tail`). Pure helpers plus a small stateful generator the MockBackend drives with a timer.
 */

/** Options of the simulation (see the `mockLive*` URL switches in mock-options.ts). */
export interface MockLiveConfig {
  /** Interval of the simulated watcher batches, ms. */
  tickMs: number;
  /** New records per second; null = the legacy simulation (one "Live event N" per tick in the first source). */
  ratePerSec: number | null;
  /** A rotation of the tailed file every this many ms (0 = never). */
  rotateEveryMs: number;
  /** An alert every this many ms (0 = never). */
  alertEveryMs: number;
}

export const LEGACY_LIVE: MockLiveConfig = { tickMs: 1500, ratePerSec: null, rotateEveryMs: 0, alertEveryMs: 0 };

const NUMBERED = /^(.+\.[A-Za-z][A-Za-z0-9]*)\.\d{1,4}$/;
const STAMPED = /[-_.]?(?:\d{4}[-_.]?\d{2}[-_.]?\d{2}(?:[T_\-.]?\d{2}(?:[-_.]?\d{2}){0,2})?|\d{10})(?:[-_.]\d{1,4})?(\.[A-Za-z][A-Za-z0-9]*)?$/;
const SEQUENCE = /(?:\.\d{1,4}|_\d{3})(\.[A-Za-z][A-Za-z0-9]*)$/;

/** The family name of a log file (mirrors `rolling::family`): rotation numbers, dates and compression suffixes removed. */
export function mockFamily(fileName: string): string {
  let name = fileName;
  const lower = name.toLowerCase();
  for (const ext of ['.gz', '.zip', '.bz2']) {
    if (lower.endsWith(ext) && name.length > ext.length) {
      name = name.slice(0, -ext.length);
      break;
    }
  }
  name = name.replace(NUMBERED, '$1');
  const stamp = STAMPED.exec(name);
  if (stamp && stamp.index > 0) name = name.slice(0, stamp.index) + (stamp[1] ?? '');
  const seq = SEQUENCE.exec(name);
  if (seq && seq.index > 0) name = name.slice(0, seq.index) + seq[1];
  return name;
}

/** The group of a source path (mirrors `rolling::group_of`): the folder kept, the file name reduced to its family. */
export function mockGroupOf(path: string): string {
  const i = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
  return i >= 0 ? `${path.slice(0, i)}/${mockFamily(path.slice(i + 1))}` : mockFamily(path);
}

/** Resolves a sliding window into `from`, as the engine does (mock approximation: "now" is the newest record). */
export function mockResolveWindow<Q extends Query>(q: Q, now: number | null): Q {
  if (!q.window || q.window.kind !== 'live') return q;
  const since = (now ?? Date.now()) - Math.max(0, q.window.lastMs);
  const { window: _window, ...rest } = q;
  return { ...rest, from: q.from == null ? since : Math.max(q.from, since) } as Q;
}

export function emptyReport(): TailReport {
  return {
    appended: 0,
    updated: 0,
    sourcesChanged: [],
    added: [],
    rotated: [],
    truncated: [],
    deleted: [],
    reimported: [],
    evicted: 0,
    idsChangedFrom: null,
    newestTs: null,
    instancesChanged: [],
    fileStates: [],
    alertsRaised: 0,
    elapsedMs: 0,
  };
}

/** What a tailed source looks like to the simulation. */
interface FileState {
  state: TailFileState;
  bytes: number;
  lines: number;
  /** Recent lines per second (smoothed). */
  rate: number;
  lastEvent: number | null;
}

const LEVEL_CYCLE = ['Information', 'Information', 'Information', 'Debug', 'Information', 'Warning', 'Information', 'Error'] as const;
const IDENTITY = ['Application', 'MachineName', 'ProcessId', 'EnvironmentName'] as const;
const BYTES_PER_LINE = 140;

/** One simulated batch: the records to append and what to report. */
export interface MockLiveBatch {
  records: MockRecord[];
  /** Sources renamed by rotation (old id → new path), and the sources created for the new files. */
  rotations: { sourceId: number; from: string; to: string; replacement: MockSource }[];
  alerts: LiveAlert[];
}

/**
 * Generates live batches over the demo sources. The data clock advances by `tickMs` per tick from the newest
 * record, so the live window and "follow now" behave as with a real file being written.
 */
export class MockLiveSim {
  private files = new Map<number, FileState>();
  private elapsed = 0;
  private sinceRotation = 0;
  private sinceAlert = 0;
  private carry = 0;
  private counter = 0;
  private alertCount = 0;
  /** The source new lines go to first (start-at-end opens and rotations retarget it). */
  private target: number | null = null;

  constructor(
    readonly config: MockLiveConfig,
    private readonly nextSourceId: () => number,
  ) {}

  /** New lines go to this source first (a file opened with "start at end"). */
  follow(sourceId: number): void {
    this.target = sourceId;
  }

  /** Records appended by live tail since the start (for messages "Live event N"). */
  get produced(): number {
    return this.counter;
  }

  /** Simulates one watcher batch. `last` gives the newest record of a source (for its instance fields and line). */
  tick(sources: MockSource[], newest: number | null, last: (sourceId: number) => MockRecord | undefined, wallNow: number): MockLiveBatch {
    const cfg = this.config;
    this.elapsed += cfg.tickMs;
    const live = sources.filter((s) => s.importer !== 'registry');
    const batch: MockLiveBatch = { records: [], rotations: [], alerts: [] };
    if (!live.length) return batch;
    const base = newest ?? Date.UTC(2026, 9, 14, 9, 0, 0);

    let count: number;
    if (cfg.ratePerSec === null) {
      count = 1;
    } else {
      this.carry += (cfg.ratePerSec * cfg.tickMs) / 1000;
      count = Math.floor(this.carry);
      this.carry -= count;
    }
    const targets = this.targets(live);
    for (let i = 0; i < count; i++) {
      const source = targets[i % targets.length];
      const ts = cfg.ratePerSec === null ? base + 1000 : base + Math.max(1, Math.round(((i + 1) * cfg.tickMs) / Math.max(1, count)));
      batch.records.push(this.record(source, ts, last(source.id), i));
    }
    // A process stats line per tick for the CPU / working set charts (realistic mode only).
    if (cfg.ratePerSec !== null && targets.length) {
      const source = targets[0];
      const prev = last(source.id);
      const cpu = (25 + 20 * Math.sin(this.elapsed / 7000) + (this.counter % 7)).toFixed(1);
      const ws = Math.round(400 + (this.elapsed / 1000) * 2);
      batch.records.push({
        ...this.record(source, base + cfg.tickMs, prev, count),
        level: 'Information',
        message: `Process stats CPU ${cpu}% · WorkingSet ${ws} MB · Threads ${20 + (this.counter % 9)}`,
        template: 'Process stats CPU {Cpu}% · WorkingSet {WorkingSet} · Threads {Threads}',
        fields: { ...identity(prev), SourceContext: 'Diagnostics.ProcessMonitor', Cpu: Number(cpu), WorkingSet: `${ws} MB` },
      });
    }
    this.account(batch.records, wallNow);

    if (cfg.rotateEveryMs > 0) {
      this.sinceRotation += cfg.tickMs;
      if (this.sinceRotation >= cfg.rotateEveryMs) {
        this.sinceRotation = 0;
        const rotated = targets[0];
        const n = sources.filter((s) => mockGroupOf(s.path) === mockGroupOf(rotated.path)).length;
        const to = `${rotated.path}.${n}`;
        const replacement: MockSource = { id: this.nextSourceId(), path: rotated.path, name: rotated.name, importer: rotated.importer, skipped: 0 };
        batch.rotations.push({ sourceId: rotated.id, from: rotated.path, to, replacement });
        this.fileOf(rotated.id).state = { kind: 'rotated' };
        this.files.set(replacement.id, { state: { kind: 'tailing' }, bytes: 0, lines: 0, rate: 0, lastEvent: wallNow });
        this.target = replacement.id;
      }
    }

    if (cfg.alertEveryMs > 0) {
      this.sinceAlert += cfg.tickMs;
      if (this.sinceAlert >= cfg.alertEveryMs) {
        this.sinceAlert = 0;
        batch.alerts.push(this.alert(base, targets[0], last(targets[0].id)));
      }
    }
    return batch;
  }

  /** The per-file status table, as `tail_status` returns it. */
  status(sources: MockSource[], records: (sourceId: number) => number, mode: WatchMode, evicted: number, total: number): TailStatus {
    const files: TailFileStatus[] = sources.map((s) => {
      const f = this.fileOf(s.id);
      const whole = s.importer === 'registry';
      const size = Math.max(f.bytes, records(s.id) * BYTES_PER_LINE);
      return {
        sourceId: s.id,
        path: s.path,
        name: s.name,
        group: mockGroupOf(s.path),
        mode: whole ? 'off' : mode,
        offset: size,
        size,
        linesPerSec: Math.round(f.rate * 10) / 10,
        lastEvent: f.lastEvent,
        state: whole ? { kind: 'whole' } : f.state,
      };
    });
    const tailing = files.filter((f) => f.state.kind === 'tailing').length;
    return {
      files,
      totals: {
        files: files.length,
        tailing,
        errors: files.filter((f) => f.state.kind === 'error').length,
        bytes: files.reduce((a, f) => a + f.size, 0),
        linesPerSec: Math.round(files.reduce((a, f) => a + f.linesPerSec, 0) * 10) / 10,
        evicted,
        records: total,
      },
    };
  }

  /** Lets the rates decay when live tail is off or quiet. */
  idle(): void {
    for (const f of this.files.values()) f.rate = 0;
  }

  private targets(live: MockSource[]): MockSource[] {
    const rotated = new Set([...this.files.entries()].filter(([, f]) => f.state.kind === 'rotated').map(([id]) => id));
    const open = live.filter((s) => !rotated.has(s.id));
    const pool = open.length ? open : live;
    const first = pool.find((s) => s.id === this.target);
    if (this.config.ratePerSec === null) return [first ?? pool[0]];
    const text = pool.filter((s) => s !== first && s.importer !== 'csv').slice(0, 2);
    return first ? [first, ...text] : text.length ? text : pool.slice(0, 1);
  }

  private record(source: MockSource, ts: number, prev: MockRecord | undefined, i: number): MockRecord {
    this.counter++;
    const level = this.config.ratePerSec === null ? 'Information' : LEVEL_CYCLE[(this.counter + i) % LEVEL_CYCLE.length];
    return {
      id: 0,
      ts,
      level,
      message: `Live event ${this.counter}`,
      template: null,
      exception: level === 'Error' ? 'System.TimeoutException: simulated live failure\n   at Live.Tail()' : null,
      fields: this.config.ratePerSec === null ? { Live: true } : { ...identity(prev), SourceContext: 'Live.Tail', Live: true, ElapsedMs: 20 + (this.counter % 400) },
      source: source.id,
      line: (prev?.line ?? 0) + 1,
      markers: [],
    };
  }

  private account(records: MockRecord[], wallNow: number): void {
    const per = new Map<number, number>();
    for (const r of records) per.set(r.source, (per.get(r.source) ?? 0) + 1);
    const secs = this.config.tickMs / 1000;
    for (const [id, f] of this.files) if (!per.has(id)) f.rate = f.rate * 0.5;
    for (const [id, n] of per) {
      const f = this.fileOf(id);
      f.lines += n;
      f.bytes += n * BYTES_PER_LINE;
      f.rate = f.rate * 0.5 + (n / secs) * 0.5;
      f.lastEvent = wallNow;
    }
  }

  private alert(now: number, source: MockSource, prev: MockRecord | undefined): LiveAlert {
    const instance = prev ? [prev.fields['MachineName'], prev.fields['ProcessId']].filter((v) => v !== undefined).join(' · ') || source.name : source.name;
    const kinds: Pick<LiveAlert, 'kind' | 'rule' | 'severity' | 'message'>[] = [
      { kind: 'errorSpike', rule: null, severity: 'critical', message: `Error spike in ${instance}: 12 errors in the last minute (usually 1)` },
      { kind: 'rule', rule: 'payment errors', severity: 'warning', message: 'Rule "payment errors": 5 matching events within 1m' },
      { kind: 'silence', rule: null, severity: 'info', message: `${instance} has been silent for 60s` },
    ];
    const pick = kinds[this.alertCount++ % kinds.length];
    const from = now - 60_000;
    const who = pick.kind === 'rule' ? (pick.rule ?? '') : instance;
    return {
      ...pick,
      // Same shape as the engine's stable id (kind:who:bucket); the mock's bucket is `from` itself, so every
      // simulated alert is distinct (the engine's cooldown would not raise them this often).
      id: `${pick.kind}:${who}:${from}`,
      instance: pick.kind === 'rule' ? null : instance,
      from,
      to: now,
      evidenceQuery: {
        from,
        to: now,
        levels: pick.kind === 'errorSpike' ? ['Error', 'Fatal'] : [],
        text: '',
        regex: false,
        caseSensitive: false,
        filters: [],
        groupBy: [],
        groups: [],
        markers: [],
        ql: pick.kind === 'rule' ? 'level = "Error"' : '',
      },
    };
  }

  private fileOf(id: number): FileState {
    let f = this.files.get(id);
    if (!f) this.files.set(id, (f = { state: { kind: 'tailing' }, bytes: 0, lines: 0, rate: 0, lastEvent: null }));
    return f;
  }
}

function identity(prev: MockRecord | undefined): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const k of IDENTITY) if (prev?.fields[k] !== undefined) out[k] = prev.fields[k];
  return out;
}

/** The watch mode files report for the requested preference (polling when asked, events otherwise). */
export function mockWatchMode(enabled: boolean, options: LiveOptions | undefined): WatchMode {
  if (!enabled) return 'off';
  return options?.watch === 'polling' ? 'polling' : 'events';
}

/** The options in effect, as `live_state` reports them: defaults filled in, values clamped like the engine and shell do. */
export function mockResolveLiveOptions(options: LiveOptions | undefined): LiveOptions {
  const o = options ?? {};
  const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
  return {
    watch: o.watch ?? 'auto',
    pollIntervalMs: clamp(o.pollIntervalMs ?? 2000, 100, 600_000),
    deletedFiles: o.deletedFiles ?? 'keep',
    retention: { maxRecords: o.retention?.maxRecords ?? null, maxAgeMs: o.retention?.maxAgeMs ?? null },
    carryFlushMs: o.carryFlushMs ?? 2000,
    detectEveryMs: clamp(o.detectEveryMs ?? 1000, 0, MAX_DETECT_EVERY_MS),
  };
}

const targetKey = (t: AlertTarget): string => ('rule' in t ? `rule:${t.rule}` : t.kind);

/** Engine-side alert mutes (mock of `workspace::tail::mutes`): validated like the engine, timed on the wall clock. */
export class MockAlertMutes {
  private readonly mutes = new Map<string, number>();

  constructor(private readonly now: () => number = Date.now) {}

  /** Throws the engine's messages for an invalid mute. */
  mute(target: AlertTarget, untilMs: number): void {
    if ('rule' in target && !target.rule.trim()) throw new Error('name the alert rule to mute');
    const now = this.now();
    if (!(untilMs > now)) throw new Error('the mute must end in the future');
    if (untilMs - now > MAX_MUTE_MS) throw new Error('alerts can be muted for at most 24 hours');
    this.mutes.set(targetKey(target), untilMs);
  }

  unmute(target: AlertTarget): void {
    this.mutes.delete(targetKey(target));
  }

  /** Whether the engine would hold this alert back. */
  covers(alert: Pick<LiveAlert, 'kind' | 'rule'>): boolean {
    const key = alert.kind === 'rule' ? `rule:${alert.rule ?? ''}` : alert.kind;
    return (this.mutes.get(key) ?? 0) > this.now();
  }
}
