import { computed, effect, inject, Injectable, signal, untracked } from '@angular/core';
import { Logger } from '../core/logger';
import { LifecycleBackend } from '../core/tools-backend';
import { UiStateRegistry } from './ui-state';
import { ReportStore } from '../share/report/report.store';

/** Quiet period before a changed state is written. */
export const AUTOSAVE_MS = 10_000;

/**
 * Autosaves the session (open files, profile and the serialized UI state) for crash recovery:
 * at most every 10 s while the state keeps changing, and immediately when the window loses focus.
 * Nothing is written while `enabled` is false (e.g. while the restore offer is still pending, so
 * the empty startup state cannot replace the session being offered).
 */
@Injectable({ providedIn: 'root' })
export class SessionService {
  private readonly backend = inject(LifecycleBackend);
  private readonly registry = inject(UiStateRegistry);
  private readonly logger = inject(Logger);
  private readonly reports = inject(ReportStore);
  /** The UI state and the reports, as one string (what is compared and written). */
  private readonly state = computed(() => `${this.registry.snapshot()}\u0000${this.reports.json()}`);

  readonly enabled = signal(true);
  /** Last time a save succeeded (epoch ms), for display. */
  readonly lastSaved = signal<number | null>(null);

  private savedSnapshot: string | null = null;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private started = false;

  constructor() {
    effect(() => {
      const snapshot = this.state();
      const enabled = this.enabled();
      untracked(() => this.changed(snapshot, enabled));
    });
  }

  /** Starts reacting to changes and window blur. Idempotent. */
  start(): void {
    if (this.started) return;
    this.started = true;
    window.addEventListener('blur', () => void this.flush());
  }

  /** Writes the session now when something changed since the last write. Returns true when it wrote. */
  async flush(): Promise<boolean> {
    clearTimeout(this.timer);
    this.timer = undefined;
    const snapshot = this.state();
    if (!this.started || !this.enabled() || snapshot === this.savedSnapshot) return false;
    return this.write(snapshot);
  }

  /** Writes the session unconditionally (before a restart); returns false when it could not be written. */
  async saveNow(): Promise<boolean> {
    clearTimeout(this.timer);
    this.timer = undefined;
    return this.write(this.state());
  }

  private changed(snapshot: string, enabled: boolean): void {
    if (this.savedSnapshot === null) this.savedSnapshot = snapshot; // the startup state is not worth saving
    if (!this.started || !enabled || snapshot === this.savedSnapshot || this.timer !== undefined) return;
    this.timer = setTimeout(() => void this.flush(), AUTOSAVE_MS);
  }

  private async write(snapshot: string): Promise<boolean> {
    try {
      const [ui = '', reports = ''] = snapshot.split('\u0000');
      await this.backend.saveSession(JSON.parse(ui), JSON.parse(reports));
      this.savedSnapshot = snapshot;
      this.lastSaved.set(Date.now());
      return true;
    } catch (e) {
      this.logger.warn('ui.session', 'Session autosave failed', { Error: e instanceof Error ? e.message : String(e) });
      return false;
    }
  }
}
