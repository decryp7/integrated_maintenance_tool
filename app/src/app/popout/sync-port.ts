import { SyncMessage } from './sync-protocol';

/** Event name of UI state sync messages between a host window and its pop-outs (Tauri adapter). */
export const UI_SYNC_EVENT = 'imt://ui-sync';

/**
 * A transport for sync messages within one window group (a host and its pop-outs). Two adapters: Tauri events
 * addressed by window label, and a BroadcastChannel in the browser build. Messages are untrusted on receipt
 * (`parseSyncMessage`).
 */
export interface SyncPort {
  post(message: SyncMessage): void;
  /** Subscribes to messages from the other windows of the group. Returns the unsubscribe function. */
  listen(handler: (message: unknown) => void): () => void;
  /** True when messages only reach the host, which must forward pop-out messages to the other pop-outs. */
  readonly needsRelay: boolean;
  close(): void;
}

/** Browser adapter: every window of the group shares one channel, so nothing needs relaying. */
export class BroadcastChannelPort implements SyncPort {
  readonly needsRelay = false;
  private readonly channel: BroadcastChannel;

  constructor(group: string, create: (name: string) => BroadcastChannel = (n) => new BroadcastChannel(n)) {
    this.channel = create(`imt-ui-sync:${group}`);
  }

  post(message: SyncMessage): void {
    try {
      this.channel.postMessage(message);
    } catch {
      /* a closed channel or an uncloneable value: the other windows keep their state */
    }
  }

  listen(handler: (message: unknown) => void): () => void {
    const fn = (e: MessageEvent) => handler(e.data);
    this.channel.addEventListener('message', fn);
    return () => this.channel.removeEventListener('message', fn);
  }

  close(): void {
    this.channel.close();
  }
}

export type EmitToFn = (label: string, event: string, payload: unknown) => Promise<void>;
export type ListenFn = (event: string, handler: (payload: unknown) => void) => Promise<() => void>;

/**
 * Tauri adapter: messages go to explicit window labels with `emitTo` (never to every window). A pop-out talks to
 * its host; the host talks to its pop-outs and relays between them (`needsRelay`).
 */
export class TauriEventPort implements SyncPort {
  readonly needsRelay: boolean;
  private readonly unlisteners: (() => void)[] = [];
  private closed = false;

  constructor(
    private readonly targets: () => readonly string[],
    private readonly emitTo: EmitToFn,
    private readonly listenFn: ListenFn,
    isHost: boolean,
  ) {
    this.needsRelay = isHost;
  }

  post(message: SyncMessage): void {
    if (this.closed) return;
    for (const label of this.targets()) void this.emitTo(label, UI_SYNC_EVENT, message).catch(() => undefined);
  }

  listen(handler: (message: unknown) => void): () => void {
    let off: (() => void) | null = null;
    let cancelled = false;
    void this.listenFn(UI_SYNC_EVENT, handler).then(
      (u) => (cancelled ? u() : (off = u)),
      () => undefined,
    );
    const stop = () => {
      cancelled = true;
      off?.();
    };
    this.unlisteners.push(stop);
    return stop;
  }

  close(): void {
    this.closed = true;
    for (const u of this.unlisteners.splice(0)) u();
  }
}
