import { inject, Injectable } from '@angular/core';
import { LiveAlert, Query } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { describeError, Logger } from '../core/logger';
import { EMPTY_QUERY, WorkspaceStore } from '../core/workspace.store';
import { TimeCursor } from '../analysis/time-cursor';
import { DockService, PANEL } from '../dock/dock.service';
import { TimelineZoom } from '../timeline/zoom';
import { AlertsStore } from './alerts.store';

/** Listens to `imt://live-alert` and applies an alert's evidence to the workspace ("Show"). */
@Injectable({ providedIn: 'root' })
export class LiveAlerts {
  private readonly backend = inject(ImtBackend);
  private readonly alerts = inject(AlertsStore);
  private readonly logger = inject(Logger);
  private readonly ws = inject(WorkspaceStore);
  private readonly zoom = inject(TimelineZoom);
  private readonly cursor = inject(TimeCursor);
  private readonly dock = inject(DockService);
  private off: (() => void) | null = null;

  /** Subscribes once (alerts only arrive while live tail is on). Never rejects. */
  async start(): Promise<void> {
    if (this.off) return;
    try {
      this.off = await this.backend.onLiveAlert((alert) => this.receive(alert));
    } catch (e) {
      this.logger.warn('ui.live', 'Live alerts unavailable', { Error: describeError(e) });
    }
  }

  stop(): void {
    this.off?.();
    this.off = null;
  }

  receive(alert: LiveAlert): void {
    const entry = this.alerts.receive(alert);
    if (!entry) return;
    this.logger.info('ui.live', 'Live alert', { Kind: alert.kind, Severity: alert.severity, Muted: entry.muted });
  }

  /**
   * Shows the evidence: the alert's query replaces the current one, the timeline zooms to its time range (which
   * pauses following) and the events grid is brought to the front.
   */
  show(alert: LiveAlert): void {
    const evidence = alert.evidenceQuery;
    const from = evidence.from ?? alert.from;
    const to = Math.max(evidence.to ?? alert.to, from + 1);
    const query: Query = { ...EMPTY_QUERY, ...evidence, from, to, window: undefined };
    this.ws.patchQuery(query);
    if (this.ws.textInput() !== query.text) this.ws.setTextInput(query.text);
    this.zoom.apply({ from, to });
    this.cursor.select({ from, to });
    this.dock.show(PANEL.events);
    this.logger.info('ui.live', 'Live alert evidence shown', { Kind: alert.kind });
  }
}
