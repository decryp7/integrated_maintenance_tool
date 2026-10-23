import { inject, Injectable, Injector } from '@angular/core';
import { DebriefEvents } from '../extensions/events';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { Logger } from '../core/logger';
import { DiagnosticsReportStore } from './diagnostics-report.store';

/** The id of the view that shows an opened diagnostics report. */
export const DIAGNOSTICS_REPORT_VIEW = 'diagnostics-report';

/**
 * Watches the workspace for an opened Debrief diagnostics zip. When the engine recognises one, the report is loaded
 * into the store and its view is revealed; a zip that looked like a report but could not be read is announced once.
 */
@Injectable({ providedIn: 'root' })
export class DiagnosticsReportService {
  private readonly events = inject(DebriefEvents);
  private readonly store = inject(DiagnosticsReportStore);
  private readonly logger = inject(Logger);
  private readonly injector = inject(Injector);
  private shown: string | null = null;
  private lastNotice: string | null = null;
  private started = false;

  /** Starts following workspace changes (idempotent). */
  start(): void {
    if (this.started) return;
    this.started = true;
    this.events.workspaceChanged.subscribe(() => void this.check());
  }

  /** Reads the report state once and reacts to it. Never throws. */
  async check(): Promise<void> {
    try {
      const state = await this.store.refresh();
      if (!state) return;
      // The context is looked up late: it depends on the workspace store, which depends on the backends.
      const ctx = this.injector.get(COMMAND_CONTEXT);
      const name = state.report?.name ?? null;
      if (name !== this.shown) {
        this.shown = name;
        if (name) {
          void this.store.loadImages();
          ctx.showView(DIAGNOSTICS_REPORT_VIEW);
        }
      }
      if (state.notice && state.notice !== this.lastNotice) ctx.notifyError(state.notice);
      this.lastNotice = state.notice;
    } catch (e) {
      this.logger.warn('ui.diagnostics', `could not check for a diagnostics report: ${String(e)}`);
    }
  }
}
