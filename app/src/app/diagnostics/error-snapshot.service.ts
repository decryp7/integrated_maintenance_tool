import { inject, Injectable, InjectionToken, signal } from '@angular/core';
import { FailureReporter } from '../core/failure-reporter';
import { describeError, Logger, redactPaths } from '../core/logger';
import { BackendFailureClassifier, DEFAULT_CLASSIFIER } from './error-classify';
import { SnapshotBackend, Unlisten } from './snapshot-backend';
import { SnapshotGate } from './snapshot-gate';
import { CapturedImage, CAPTURING_ATTRIBUTE, NO_SCREENSHOT_IMAGE, noScreenshotNote } from './snapshot.types';
import { EngineError, SnapshotContext, SnapshotReason } from '../core/api.types';

/** Time sources, replaceable in tests. */
export interface SnapshotClock {
  now(): number;
  /** Resolves after the next animation frame, so the error UI is on screen. */
  nextFrame(): Promise<void>;
}

export const SNAPSHOT_CLOCK = new InjectionToken<SnapshotClock>('SNAPSHOT_CLOCK', {
  providedIn: 'root',
  factory: () => ({
    now: () => Date.now(),
    nextFrame: () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())),
  }),
});

/**
 * Optional filter for the text a snapshot stores (message, stack): the share feature provides one that
 * passes it through the engine's redaction while the window's redacted view is on.
 */
export type SnapshotTextFilter = (text: string) => Promise<string>;
export const SNAPSHOT_TEXT_FILTER = new InjectionToken<SnapshotTextFilter>('SNAPSHOT_TEXT_FILTER');

/** The small notice shown after a capture. */
export interface SnapshotToast {
  kind: 'captured' | 'failed';
  id: string | null;
  /** First snapshot ever: also show that it is saved locally and nothing is sent. */
  firstTime: boolean;
  /** Taken by the capture command rather than after an error. */
  manual?: boolean;
  /** The panel that failed, when one did ("Something went wrong in Timeline."). */
  where?: string;
}

const CATEGORY = 'ui.snapshot';

/** Details a trigger can add. */
export interface CaptureDetail {
  message?: string;
  error?: unknown;
  /** The panel that failed, for the toast. */
  where?: string;
}

/**
 * Takes error snapshots: every trigger funnels through {@link capture}. It waits one frame, throttles
 * (one per 10 s, 20 per session; the manual command is exempt), saves through the backend and never
 * throws or re-enters itself: errors raised while capturing are logged and ignored.
 */
@Injectable({ providedIn: 'root' })
export class ErrorSnapshotService implements FailureReporter {
  private readonly backend = inject(SnapshotBackend);
  private readonly logger = inject(Logger);
  private readonly clock = inject(SNAPSHOT_CLOCK);
  private readonly gate = new SnapshotGate();
  private readonly textFilter = inject(SNAPSHOT_TEXT_FILTER, { optional: true });
  private busy = false;
  private automatic = true;
  private unlisten: Unlisten | null = null;

  /** The latest capture result for the toast; null when dismissed. */
  readonly toast = signal<SnapshotToast | null>(null);
  /** Increments after each saved snapshot (views refresh on it). */
  readonly saved = signal(0);

  /** Starts listening for engine errors and reads whether automatic snapshots are on. Safe to call twice. */
  async start(): Promise<void> {
    if (this.unlisten) return;
    this.unlisten = () => undefined;
    try {
      this.automatic = await this.backend.automaticEnabled();
      this.unlisten = await this.backend.onEngineError((e) => this.engineError(e));
    } catch (e) {
      this.logger.warn(CATEGORY, 'Could not start error snapshots', { Error: redactPaths(describeError(e)) });
    }
  }

  stop(): void {
    this.unlisten?.();
    this.unlisten = null;
  }

  /** Re-reads the on/off setting (call after the settings change). */
  async refreshEnabled(): Promise<void> {
    try {
      this.automatic = await this.backend.automaticEnabled();
    } catch {
      /* keep the last known value */
    }
  }

  dismissToast(): void {
    this.toast.set(null);
  }

  /** The user pressed the capture command. */
  manual(): Promise<string | null> {
    return this.capture('manual', {});
  }

  /** From the Angular `ErrorHandler`: unhandled errors and promise rejections. */
  unhandled(error: unknown): void {
    void this.capture('unhandled-error', { error });
  }

  /** From a panel's error boundary when it shows its error card. */
  panelFailed(panel: string, error: unknown): void {
    void this.capture('panel-failed', { error, where: panel, message: `Panel "${panel}" failed: ${describeError(error)}` });
  }

  /** From a failed backend command. User mistakes (validation, syntax, ...) are skipped unless `classify` says otherwise. */
  backendFailed(command: string, error: unknown, classify: BackendFailureClassifier = DEFAULT_CLASSIFIER): void {
    if (!classify(command, error)) return;
    void this.capture('backend-failed', { error, message: `Command ${command} failed: ${describeError(error)}` });
  }

  /** From an `imt://error` event. */
  engineError(e: EngineError): void {
    void this.capture('engine-error', { message: `${e.kind}${e.plugin ? ` (${e.plugin})` : ''}: ${e.message}` });
  }

  /** Takes a snapshot now. Resolves to its id, or null when skipped (throttled, off, busy) or failed. Never rejects. */
  async capture(reason: SnapshotReason, detail: CaptureDetail = {}): Promise<string | null> {
    const manual = reason === 'manual';
    // Errors raised while capturing must not start another capture.
    if (this.busy || (!manual && !this.automatic) || !this.gate.tryAcquire(this.clock.now(), manual)) return null;
    this.busy = true;
    try {
      const { image, note } = await this.screenshot();
      const context = this.context(reason, detail, manual);
      if (note) context.message = `${context.message}\n${note}`;
      const result = await this.backend.save(image, await this.filtered(context));
      this.saved.update((n) => n + 1);
      this.toast.set({ kind: 'captured', id: result.id, firstTime: result.firstTime, ...(manual ? { manual } : {}), ...(detail.where ? { where: detail.where } : {}) });
      return result.id;
    } catch (e) {
      this.logger.warn(CATEGORY, 'Could not capture an error snapshot', { Reason: reason, Error: redactPaths(describeError(e)) });
      if (manual) this.toast.set({ kind: 'failed', id: null, firstTime: false });
      return null;
    } finally {
      this.busy = false;
    }
  }

  /**
   * The window as the shell captures it natively (ADR 0034): nothing is rasterised in the page, so the UI keeps
   * running while the webview takes its picture. Toasts are hidden for that frame. Without a picture the snapshot
   * keeps a blank image and a note saying why.
   */
  private async screenshot(): Promise<{ image: CapturedImage; note?: string }> {
    const root = document.documentElement;
    root.setAttribute(CAPTURING_ATTRIBUTE, '');
    try {
      await this.clock.nextFrame();
      const shot = await this.backend.captureWindow();
      if (shot.status === 'captured') return { image: { base64: shot.image, mime: shot.mime, bytes: shot.bytes } };
      const why = redactPaths(shot.reason);
      this.logger.info(CATEGORY, 'No screenshot for the error snapshot', { Reason: why });
      return { image: NO_SCREENSHOT_IMAGE, note: noScreenshotNote(why) };
    } finally {
      root.removeAttribute(CAPTURING_ATTRIBUTE);
    }
  }

  /** The context with its texts passed through the optional filter (redaction). */
  private async filtered(context: SnapshotContext): Promise<SnapshotContext> {
    const filter = this.textFilter;
    if (!filter) return context;
    return { ...context, message: await filter(context.message), stack: context.stack === undefined ? undefined : await filter(context.stack) };
  }

  private context(reason: SnapshotReason, detail: CaptureDetail, manual: boolean): SnapshotContext {
    const error = detail.error;
    const message = detail.message ?? (error === undefined ? reason : describeError(error));
    const stack = error instanceof Error ? error.stack : undefined;
    return {
      reason,
      message: redactPaths(message),
      stack: stack ? redactPaths(stack) : undefined,
      view: activeViewTitle(),
      manual,
    };
  }
}

/** Title of the active dock tab (best effort). */
function activeViewTitle(): string | undefined {
  const text = document.querySelector('.dv-tab.dv-active-tab, .dv-tab.active-tab')?.textContent?.trim();
  return text || undefined;
}
