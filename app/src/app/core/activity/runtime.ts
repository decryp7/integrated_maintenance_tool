import { Detail, safeParams } from './privacy';
import { TriggerTracker } from './trigger';
import { ACTIVITY_CONTEXT, ActivityParams, Breadcrumb, Outcome, RING_SIZE, Trigger, USER_TEMPLATE } from './types';

/**
 * The activity log of this window, framework-free so the logger, the backend proxy and the IPC layer can use
 * it without dependency injection (one JS realm = one window = one runtime). `ActivityLog` is its Angular face.
 *
 * An operation is "current" while it runs and for a short linger afterwards, so the backend calls and log
 * events it causes (including from effects that run right after) carry its id. JavaScript has no async
 * context, so concurrent operations attribute to the most recent one: good enough to correlate, and the
 * Rust side logs every IPC call regardless.
 */

/** Where entries go (the Logger, bound by `ActivityLog`). */
export type ActivitySink = (level: 'info' | 'warn', message: string, props: Record<string, string>) => void;

export interface Clock {
  now(): number;
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}

const REAL_CLOCK: Clock = {
  now: () => Date.now(),
  setTimeout: (fn, ms) => setTimeout(fn, ms),
  clearTimeout: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
};

/** How long an operation stays current after it ended. */
export const LINGER_MS = 1000;
/** Entries buffered before a sink is bound. */
const MAX_BUFFERED = 100;
/** Most coalescing keys pending at once. */
const MAX_COALESCED = 32;

/** A running operation. */
export interface Operation {
  readonly id: string;
  readonly operation: string;
  /** `auto`: resolved from the latest user input when the operation starts (see `TriggerTracker`). */
  readonly trigger: Trigger;
  readonly params: Record<string, string>;
  readonly started: number;
  readonly view?: string;
  /** User inputs counted when it started (see `attributable`). */
  readonly inputs: number;
  /** A backend call nobody named: state changes it causes are still the user's own operations. */
  readonly weak?: boolean;
}

interface Coalesced {
  op: Operation;
  count: number;
  timer: unknown;
  params: ActivityParams;
}

export function newOperationId(): string {
  const c = globalThis.crypto as Crypto | undefined;
  if (c?.randomUUID) return c.randomUUID();
  return 'xxxxxxxx-xxxx-4xxx-8xxx-xxxxxxxxxxxx'.replace(/x/g, () => Math.floor(Math.random() * 16).toString(16));
}

/** A short code for an error: its `code` when it has one (E_IO, NOT_FOUND), else its name, else E_FAILED. */
export function errorCode(e: unknown): string {
  const code = (e as { code?: unknown } | null)?.code;
  if (typeof code === 'string' && /^[A-Za-z][\w.-]{0,40}$/.test(code)) return code;
  const text = typeof e === 'string' ? e : e instanceof Error ? e.message : '';
  const inText = /\b(E_[A-Z0-9_]{2,40})\b/.exec(text);
  const coded = inText?.[1];
  if (coded) return coded;
  if (e instanceof Error && e.name && e.name !== 'Error') return e.name;
  return 'E_FAILED';
}

/** True for an error that means the user (or the app on their behalf) cancelled. */
export function isCancellation(e: unknown): boolean {
  if ((e as { name?: unknown } | null)?.name === 'AbortError') return true;
  const code = (e as { code?: unknown } | null)?.code;
  if (code === 'CANCELLED' || code === 'E_CANCELLED') return true;
  const text = typeof e === 'string' ? e : e instanceof Error ? e.message : '';
  return /\b(cancel+ed|aborted)\b/i.test(text);
}

/** The label of this window: the Tauri window, else `main` (or `popout` for a pop-out page in a browser). */
export function detectWindowLabel(): string {
  const label = (globalThis as { __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } } }).__TAURI_INTERNALS__?.metadata?.currentWindow
    ?.label;
  if (typeof label === 'string' && label) return label;
  return typeof location !== 'undefined' && location.hash.startsWith('#/popout') ? 'popout' : 'main';
}

/** Title of the active dock tab (best effort). */
export function activeViewTitle(): string | undefined {
  if (typeof document === 'undefined') return undefined;
  const tab =
    document.querySelector('.dv-active-group .dv-tab.dv-active-tab') ?? document.querySelector('.dv-tab.dv-active-tab, .dv-tab.active-tab');
  const text = (tab?.querySelector('.imt-tab-label') ?? tab)?.textContent?.trim();
  return text ? text.slice(0, 80) : undefined;
}

export class ActivityRuntime {
  detail: Detail = 'normal';
  window = detectWindowLabel();
  view: () => string | undefined = activeViewTitle;
  readonly triggers: TriggerTracker;
  private sink: ActivitySink | null = null;
  private buffered: [level: 'info' | 'warn', message: string, props: Record<string, string>][] = [];
  private readonly running: Operation[] = [];
  private lingering: { op: Operation; until: number } | null = null;
  private ring: Breadcrumb[] = [];
  private readonly coalesced = new Map<string, Coalesced>();
  private readonly listeners = new Set<(b: Breadcrumb) => void>();

  constructor(private readonly clock: Clock = REAL_CLOCK) {
    this.triggers = new TriggerTracker(() => clock.now());
  }

  /** Binds where entries go; entries recorded before are flushed to it. */
  setSink(sink: ActivitySink | null): void {
    this.sink = sink;
    if (!sink) return;
    const buffered = this.buffered;
    this.buffered = [];
    for (const [level, message, props] of buffered) this.safeEmit(level, message, props);
  }

  /** Called with every entry (after it is logged). Returns the unsubscribe function. */
  subscribe(fn: (b: Breadcrumb) => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** The operation running now, or the one that just ended. */
  current(): Operation | undefined {
    const running = this.running[this.running.length - 1];
    if (running) return running;
    if (this.lingering && this.clock.now() <= this.lingering.until) return this.lingering.op;
    return undefined;
  }

  /**
   * The operation a new call belongs to: the running one, or the one that just ended unless the user did
   * something since it started (then the call is a new operation of its own).
   */
  attributable(): Operation | undefined {
    const running = this.running[this.running.length - 1];
    if (running) return running;
    const l = this.lingering;
    if (l && this.clock.now() <= l.until && this.triggers.inputCount() === l.op.inputs) return l.op;
    return undefined;
  }

  currentOperationId(): string | undefined {
    return this.current()?.id;
  }

  /** Starts an operation; end it with `end`. */
  begin(operation: string, trigger: Trigger | 'auto', params: ActivityParams = {}, options: { weak?: boolean } = {}): Operation {
    const op: Operation = {
      ...(options.weak ? { weak: true } : {}),
      id: newOperationId(),
      operation,
      trigger: trigger === 'auto' ? this.triggers.infer() : trigger,
      params: safeParams(params, this.detail),
      started: this.clock.now(),
      view: this.view(),
      inputs: this.triggers.inputCount(),
    };
    this.running.push(op);
    return op;
  }

  /** Ends an operation and logs its entry. Safe to call twice (the second call is ignored). */
  end(op: Operation, outcome: Outcome, error?: unknown, extra: ActivityParams = {}): void {
    const i = this.running.lastIndexOf(op);
    if (i < 0) return;
    this.running.splice(i, 1);
    this.lingering = { op, until: this.clock.now() + LINGER_MS };
    this.emit(op, outcome, error, extra);
  }

  /** Runs `fn` as an operation: one entry with its duration and outcome (ok, cancelled, error). Rethrows. */
  async track<T>(operation: string, trigger: Trigger | 'auto', params: ActivityParams, fn: () => T | Promise<T>): Promise<T> {
    const op = this.begin(operation, trigger, params);
    try {
      const result = await fn();
      this.end(op, 'ok');
      return result;
    } catch (e) {
      this.end(op, isCancellation(e) ? 'cancelled' : 'error', e);
      throw e;
    }
  }

  /** An instant operation (nothing to wait for). */
  record(operation: string, trigger: Trigger | 'auto', params: ActivityParams = {}, outcome: Outcome = 'ok', error?: unknown): string {
    const op = this.begin(operation, trigger, params);
    this.end(op, outcome, error);
    return op.id;
  }

  /**
   * A high-frequency change (typing a query, dragging a time range, moving the selection): the first change
   * starts an operation that is current at once (so the requests it causes carry its id); changes within
   * `delayMs` of each other are merged into one entry with `Count`, logged when they stop. Returns the id.
   */
  coalesce(key: string, operation: string, trigger: Trigger | 'auto', params: ActivityParams, delayMs: number): string {
    let c = this.coalesced.get(key);
    if (c) {
      this.clock.clearTimeout(c.timer);
      c.count++;
      c.params = params;
    } else {
      if (this.coalesced.size >= MAX_COALESCED) this.flushCoalesced();
      const op = this.begin(operation, trigger, {});
      // Not "running": it must not hide later operations; it lingers until it is logged.
      this.running.splice(this.running.lastIndexOf(op), 1);
      c = { op, count: 1, timer: null, params };
      this.coalesced.set(key, c);
    }
    this.lingering = { op: c.op, until: this.clock.now() + delayMs + LINGER_MS };
    const entry = c;
    entry.timer = this.clock.setTimeout(() => this.finishCoalesced(key, entry), delayMs);
    return entry.op.id;
  }

  /** Logs every pending coalesced change now (before an error report, or at shutdown). */
  flushCoalesced(): void {
    for (const [key, c] of [...this.coalesced]) {
      this.clock.clearTimeout(c.timer);
      this.finishCoalesced(key, c);
    }
  }

  private finishCoalesced(key: string, c: Coalesced): void {
    if (this.coalesced.get(key) !== c) return;
    this.coalesced.delete(key);
    const op: Operation = { ...c.op, params: safeParams(c.params, this.detail) };
    this.emit(op, 'ok', undefined, c.count > 1 ? { Count: c.count } : {});
  }

  /** The latest entries of this window, oldest first. */
  breadcrumbs(n = 30): Breadcrumb[] {
    return this.ring.slice(Math.max(0, this.ring.length - n));
  }

  private emit(op: Operation, outcome: Outcome, error: unknown, extra: ActivityParams): void {
    // The trigger may only become known after the start: the native menu's event can arrive late.
    const trigger = op.trigger === 'program' && this.triggers.nativeMenuSent(op.operation, op.started) ? 'native-menu' : op.trigger;
    const durationMs = Math.max(0, Math.round(this.clock.now() - op.started));
    const code = outcome === 'error' ? errorCode(error) : undefined;
    const params = { ...op.params, ...safeParams(extra, this.detail) };
    const crumb: Breadcrumb = {
      time: new Date(this.clock.now()).toISOString(),
      operation: op.operation,
      trigger,
      operationId: op.id,
      outcome,
      ...(code ? { errorCode: code } : {}),
      durationMs,
      ...(op.view ? { view: op.view } : {}),
      window: this.window,
      params,
    };
    this.ring.push(crumb);
    if (this.ring.length > RING_SIZE) this.ring.splice(0, this.ring.length - RING_SIZE);
    const props: Record<string, string> = {
      ...params,
      SourceContext: ACTIVITY_CONTEXT,
      MessageTemplate: USER_TEMPLATE,
      Operation: op.operation,
      Trigger: trigger,
      OperationId: op.id,
      Outcome: outcome,
      DurationMs: String(durationMs),
      Window: this.window,
    };
    if (code) props['ErrorCode'] = code;
    if (op.view) props['View'] = op.view;
    const message = `User ${op.operation} via ${trigger}`;
    const level = outcome === 'error' ? 'warn' : 'info';
    if (this.sink) this.safeEmit(level, message, props);
    else if (this.buffered.length < MAX_BUFFERED) this.buffered.push([level, message, props]);
    for (const l of this.listeners) {
      try {
        l(crumb);
      } catch {
        /* a listener must not break logging */
      }
    }
  }

  private safeEmit(level: 'info' | 'warn', message: string, props: Record<string, string>): void {
    try {
      this.sink?.(level, message, props);
    } catch {
      /* logging must never break the operation */
    }
  }

  /** Forgets everything (tests). */
  reset(): void {
    for (const c of this.coalesced.values()) this.clock.clearTimeout(c.timer);
    this.coalesced.clear();
    this.running.length = 0;
    this.lingering = null;
    this.ring = [];
    this.buffered = [];
    this.sink = null;
    this.listeners.clear();
    this.triggers.reset();
    this.detail = 'normal';
  }
}

/** This window's activity log. */
export const activity = new ActivityRuntime();

/** The id of the current operation (for log events and IPC headers), if any. */
export function currentOperationId(): string | undefined {
  return activity.currentOperationId();
}
