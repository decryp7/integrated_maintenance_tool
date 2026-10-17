import { DestroyRef, effect, EffectRef, inject, Injectable, Injector, untracked } from '@angular/core';
import { Logger } from '../core/logger';
import { fingerprint, MAX_MESSAGE_CHARS, newOrigin, parseSyncMessage, SyncBody, SyncEngine } from './sync-protocol';
import { SyncPort } from './sync-port';

/** A pop-out repeats its hello this often until the host answers. */
export const HELLO_INTERVAL_MS = 1000;
export const HELLO_TRIES = 10;

/** A piece of UI state shared by a host window and its pop-outs. */
export interface SyncedValue<T = unknown> {
  key: string;
  /** Reads the value (signals read here make it reactive). */
  read(): T;
  /** Applies a value from another window. Must be synchronous; untrusted input (validate). */
  apply(value: unknown): void;
  /** `fromHost`: only the host's changes are shared (data versions, live state the engine owns). Default: both ways. */
  direction?: 'both' | 'fromHost';
}

/** A command a pop-out asks its host to run (global undo, showing a view in the main layout). */
export type HostCommandHandler = (command: string, arg: string | undefined) => void;

/**
 * Cross-window UI state sync of one window group over a `SyncPort` (see `sync-protocol` for the rules). The host
 * is the source of truth; every window still queries the engine itself. Start it once per window.
 */
@Injectable({ providedIn: 'root' })
export class UiSync {
  private readonly injector = inject(Injector);
  private readonly logger = inject(Logger);
  private engine: SyncEngine | null = null;
  private port: SyncPort | null = null;
  private values = new Map<string, SyncedValue>();
  private effects: EffectRef[] = [];
  private stopListening: (() => void) | null = null;
  private commandHandler: HostCommandHandler | null = null;
  private hostGoneHandler: (() => void) | null = null;
  private syncedHandler: (() => void) | null = null;
  private synced = false;
  private helloTimer: ReturnType<typeof setInterval> | null = null;
  private label = '';

  constructor() {
    inject(DestroyRef).onDestroy(() => this.stop());
  }

  get started(): boolean {
    return this.engine !== null;
  }

  get isHost(): boolean {
    return this.engine?.isHost ?? false;
  }

  /**
   * Starts syncing `values` over `port`. A pop-out (`isHost` false) says hello and takes the host's snapshot.
   * `label` identifies this window in hello/snapshot exchanges.
   */
  start(port: SyncPort, isHost: boolean, label: string, values: readonly SyncedValue[], origin = newOrigin()): void {
    this.stop();
    this.engine = new SyncEngine(origin, isHost);
    this.port = port;
    this.label = label;
    this.values = new Map(values.map((v) => [v.key, v]));
    this.stopListening = port.listen((raw) => this.receive(raw));
    // What each value is now is not a change: only later changes are shared (a new pop-out must not push its
    // defaults over the host's state; the host's snapshot answers its hello instead).
    for (const v of values) this.engine.noteApplied(v.key, untracked(() => v.read()));
    for (const v of values) {
      this.effects.push(
        effect(
          () => {
            const value = v.read();
            untracked(() => this.local(v, value));
          },
          { injector: this.injector, manualCleanup: true },
        ),
      );
    }
    if (!isHost) this.greet(label);
  }

  /** Says hello until the host answers (it may still be starting), at most `HELLO_TRIES` times. */
  private greet(label: string): void {
    let tries = 0;
    const hello = () => {
      if (!this.engine || this.synced || tries++ >= HELLO_TRIES) return this.stopGreeting();
      this.send({ t: 'hello', label });
    };
    hello();
    this.helloTimer = setInterval(hello, HELLO_INTERVAL_MS);
  }

  private stopGreeting(): void {
    if (this.helloTimer !== null) clearInterval(this.helloTimer);
    this.helloTimer = null;
  }

  /** Host: handles commands from pop-outs. */
  onCommand(handler: HostCommandHandler): void {
    this.commandHandler = handler;
  }

  /** Pop-out: called once the host's snapshot was applied (this window is up to date). */
  onSynced(handler: () => void): void {
    this.syncedHandler = handler;
  }

  /** Pop-out: called when the host says it is going away. */
  onHostGone(handler: () => void): void {
    this.hostGoneHandler = handler;
  }

  /** Pop-out: asks the host to run a command. Returns false when sync is not running. */
  runInHost(command: string, arg?: string): boolean {
    if (!this.engine || this.engine.isHost) return false;
    this.send({ t: 'command', command, ...(arg !== undefined ? { arg } : {}) });
    return true;
  }

  /** Host: tells the pop-outs it is going away (page unload). */
  goodbye(): void {
    if (this.engine?.isHost) this.send({ t: 'bye' });
  }

  stop(): void {
    this.stopGreeting();
    this.synced = false;
    this.effects.forEach((e) => e.destroy());
    this.effects = [];
    this.stopListening?.();
    this.stopListening = null;
    this.port?.close();
    this.port = null;
    this.engine = null;
  }

  private local(v: SyncedValue, value: unknown): void {
    const engine = this.engine;
    if (!engine) return;
    if (!engine.shouldSend(v.key, value)) return;
    if (v.direction === 'fromHost' && !engine.isHost) return;
    this.send({ t: 'state', key: v.key, value });
  }

  private send(body: SyncBody): void {
    const engine = this.engine;
    if (!engine || !this.port) return;
    const message = engine.stamp(body);
    if (fingerprint(message).length > MAX_MESSAGE_CHARS) {
      this.logger.warn('ui.popout', 'UI state too large to share with pop-outs', { Kind: message.t });
      return;
    }
    this.port.post(message);
  }

  private receive(raw: unknown): void {
    const engine = this.engine;
    const msg = parseSyncMessage(raw);
    if (!engine || !msg || !engine.accept(msg)) return;
    // Tauri delivers pop-out messages to the host only; it forwards them to the other pop-outs.
    if (engine.isHost && !msg.host && this.port?.needsRelay && msg.t === 'state') this.port.post(msg);
    switch (msg.t) {
      case 'state':
        this.apply(msg.key, msg.value, msg.host);
        break;
      case 'hello':
        if (engine.isHost) this.send({ t: 'snapshot', to: msg.label, values: this.snapshot() });
        break;
      case 'snapshot':
        if (!engine.isHost && msg.host && msg.to === this.label && !this.synced) {
          for (const [k, v] of Object.entries(msg.values)) this.apply(k, v, true);
          this.synced = true;
          this.stopGreeting();
          this.syncedHandler?.();
        }
        break;
      case 'command':
        if (engine.isHost) this.commandHandler?.(msg.command, msg.arg);
        break;
      case 'bye':
        engine.forget(msg.origin);
        if (!engine.isHost && msg.host) this.hostGoneHandler?.();
        break;
    }
  }

  private snapshot(): Record<string, unknown> {
    const out: Record<string, unknown> = {};
    for (const [k, v] of this.values) out[k] = untracked(() => v.read());
    return out;
  }

  private apply(key: string, value: unknown, fromHost: boolean): void {
    const v = this.values.get(key);
    const engine = this.engine;
    if (!v || !engine) return;
    if (v.direction === 'fromHost' && !fromHost) return;
    try {
      v.apply(value);
    } catch (e) {
      this.logger.warn('ui.popout', 'Shared UI state could not be applied', { Key: key, Error: e instanceof Error ? e.message : String(e) });
    }
    // What the value is now (apply may normalise it), so observing it does not send it back.
    engine.noteApplied(key, untracked(() => v.read()));
  }
}
