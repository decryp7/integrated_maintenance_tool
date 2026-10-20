/**
 * The cross-window UI state protocol between a host window and its pop-outs (pure, no Angular, no transport).
 *
 * - Every window has a random `origin` id and numbers what it sends (`seq`, strictly increasing).
 * - A receiver drops its own messages, malformed ones and anything not newer than the last message it accepted
 *   from that origin (ordering: a late, stale update never overwrites a newer one).
 * - Echo prevention: after applying a remote value a window records what the value now is; its change observer
 *   only sends when the value differs from the last one sent or applied for that key. So applying a remote change
 *   never bounces it back.
 * - The host is the source of truth: a pop-out says `hello` on start and the host answers with a `snapshot`.
 */

export type SyncMessage =
  | { t: 'state'; origin: string; seq: number; host: boolean; key: string; value: unknown }
  | { t: 'hello'; origin: string; seq: number; host: boolean; label: string }
  | { t: 'snapshot'; origin: string; seq: number; host: boolean; to: string; values: Record<string, unknown> }
  | { t: 'command'; origin: string; seq: number; host: boolean; command: string; arg?: string }
  | { t: 'bye'; origin: string; seq: number; host: boolean };

export type SyncMessageType = SyncMessage['t'];
/** A message as written by its sender, before `SyncEngine.stamp` adds origin, sequence number and role. */
export type SyncBody = SyncMessage extends infer M ? (M extends SyncMessage ? Omit<M, 'origin' | 'seq' | 'host'> : never) : never;

const TYPES: ReadonlySet<string> = new Set<SyncMessageType>(['state', 'hello', 'snapshot', 'command', 'bye']);
/** Larger messages are dropped (a selected record with huge fields should not flood every window). */
export const MAX_MESSAGE_CHARS = 512 * 1024;

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

/** Parses a message from another window; null when it is not a well-formed sync message. */
export function parseSyncMessage(raw: unknown): SyncMessage | null {
  if (!isRecord(raw)) return null;
  const { t, origin, seq, host } = raw;
  if (typeof t !== 'string' || !TYPES.has(t) || typeof origin !== 'string' || !origin || origin.length > 64) return null;
  if (typeof seq !== 'number' || !Number.isSafeInteger(seq) || seq < 0 || typeof host !== 'boolean') return null;
  const base = { origin, seq, host };
  switch (t as SyncMessageType) {
    case 'state':
      return typeof raw['key'] === 'string' && 'value' in raw ? { t: 'state', ...base, key: raw['key'], value: raw['value'] } : null;
    case 'hello':
      return typeof raw['label'] === 'string' ? { t: 'hello', ...base, label: raw['label'] } : null;
    case 'snapshot':
      return typeof raw['to'] === 'string' && isRecord(raw['values']) ? { t: 'snapshot', ...base, to: raw['to'], values: raw['values'] } : null;
    case 'command':
      return typeof raw['command'] === 'string' && (raw['arg'] === undefined || typeof raw['arg'] === 'string')
        ? { t: 'command', ...base, command: raw['command'], ...(raw['arg'] !== undefined ? { arg: raw['arg'] as string } : {}) }
        : null;
    case 'bye':
      return { t: 'bye', ...base };
  }
}

/** A stable text form of a value for comparisons (undefined and null are the same: "no value"). */
export function fingerprint(value: unknown): string {
  return JSON.stringify(value ?? null) ?? 'null';
}

/** Ordering and echo bookkeeping of one window. */
export class SyncEngine {
  private seq = 0;
  private readonly lastSeq = new Map<string, number>();
  private readonly known = new Map<string, string>();

  constructor(
    readonly origin: string,
    readonly isHost: boolean,
  ) {}

  /** The next sequence number to send. */
  next(): number {
    return ++this.seq;
  }

  /** Builds an outgoing message with this window's origin and the next sequence number. */
  stamp(body: SyncBody): SyncMessage {
    return { ...body, origin: this.origin, seq: this.next(), host: this.isHost } as SyncMessage;
  }

  /** Should a received message be handled? Not our own, and newer than the last one from its sender. */
  accept(msg: SyncMessage): boolean {
    if (msg.origin === this.origin) return false;
    const last = this.lastSeq.get(msg.origin) ?? 0;
    if (msg.seq <= last) return false;
    this.lastSeq.set(msg.origin, msg.seq);
    return true;
  }

  /** Should a local value be sent? Only when it differs from the last value sent or applied for the key. */
  shouldSend(key: string, value: unknown): boolean {
    const fp = fingerprint(value);
    if (this.known.get(key) === fp) return false;
    this.known.set(key, fp);
    return true;
  }

  /** Records the value a key has after applying a remote change, so observing it does not send it back. */
  noteApplied(key: string, value: unknown): void {
    this.known.set(key, fingerprint(value));
  }

  /** Forgets the value last sent for a key (it was not sent: nobody listened), so the next change is sent. */
  forgetValue(key: string): void {
    this.known.delete(key);
  }

  /** Forgets a sender (it said bye), so a window reusing nothing from it starts fresh. */
  forget(origin: string): void {
    this.lastSeq.delete(origin);
  }
}

/** A random id for this window's messages. */
export function newOrigin(random: () => number = Math.random): string {
  return `o${Date.now().toString(36)}${Math.floor(random() * 2 ** 32).toString(36)}`;
}
