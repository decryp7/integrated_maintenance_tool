import { inject, Injectable, Injector } from '@angular/core';
import type { EChartsCoreOption } from 'echarts/core';
import { Anomaly, ChartDef } from '../../core/api.types';
import { AnalysisBackend } from '../../core/analysis-backend';
import { formatFull } from '../../core/format';
import { WorkspaceStore } from '../../core/workspace.store';
import { QuerySession } from '../../analysis/query-session';
import { BookmarkStore } from '../../projects/bookmarks/bookmark.store';
import { TimelineAnnotations } from '../../timeline/annotations.store';
import { plural } from '../../i18n/locale';
import { errorMessage } from '../../tools/tool-errors';
import { VisualExportService } from '../visuals/visual-export.service';
import { ShareBackend } from '../share-backend';
import { ReportStore } from './report.store';

/** Most rows copied from a query result into a report. */
export const MAX_TABLE_ROWS = 200;
/** Most anomalies added as findings at once. */
export const MAX_FINDINGS = 10;

/** How urgent an anomaly reads in a report. */
export function anomalySeverity(a: Anomaly): 'critical' | 'warning' | 'info' {
  if (a.kind === 'crash') return 'critical';
  if (a.kind === 'errorSpike' || a.kind === 'memoryGrowth') return 'warning';
  return 'info';
}

/**
 * "Add to report": copies what the user is looking at into the active report (creating one when there is
 * none). Events are referenced by id and resolved (and redacted) by the engine at export; charts and the
 * timeline are snapshotted as SVG now. Each returns false when there was nothing to add.
 */
@Injectable({ providedIn: 'root' })
export class ReportActions {
  private readonly reports = inject(ReportStore);
  private readonly store = inject(WorkspaceStore);
  private readonly bookmarks = inject(BookmarkStore);
  private readonly visuals = inject(VisualExportService);
  private readonly analysis = inject(AnalysisBackend);
  private readonly session = inject(QuerySession);
  private readonly annotations = inject(TimelineAnnotations);
  private readonly injector = inject(Injector);
  /** Resolved on use: `ShareBackend` is provided by `provideShare()`, not at the root. */
  private share(): ShareBackend {
    return this.injector.get(ShareBackend);
  }

  /** Notice that something was added; `message` is the complete, translated sentence. */
  private added(message: string): true {
    this.store.setNotice(message);
    return true;
  }

  addText(markdown = ''): boolean {
    this.reports.add({ type: 'text', markdown });
    return this.added($localize`:@@share.reportActions.addedText:Added a text block to the report`);
  }

  /** Events by id (the context menu: the record, or the grid's multi-selection). */
  addEvents(ids: readonly number[]): boolean {
    if (!ids.length) return false;
    this.reports.add({ type: 'events', caption: '', ids: [...ids] });
    return this.added(
      plural(ids.length, {
        one: $localize`:@@share.reportActions.addedEvent:Added the event to the report`,
        other: $localize`:@@share.reportActions.addedEvents:Added ${ids.length}:count: events to the report`,
      }),
    );
  }

  addSelectedEvent(): boolean {
    const r = this.store.selected();
    if (!r) return false;
    this.reports.add({ type: 'events', caption: '', ids: [r.id] });
    return this.added($localize`:@@share.reportActions.addedSelected:Added the selected event to the report`);
  }

  /** Event bookmarks become one events block; range bookmarks an annotations block. */
  addBookmarks(): boolean {
    const items = this.bookmarks.items();
    const ids = items.filter((b) => b.kind === 'record' && b.recordId !== undefined).map((b) => b.recordId!);
    const ranges = items.filter((b) => b.kind === 'range' && b.from !== undefined && b.to !== undefined);
    if (!ids.length && !ranges.length) return false;
    if (ids.length) this.reports.add({ type: 'events', caption: $localize`:@@share.reportActions.bookmarkedEvents:Bookmarked events`, ids });
    if (ranges.length) {
      this.reports.add({ type: 'annotations', caption: $localize`:@@share.reportActions.bookmarkedRanges:Bookmarked time ranges`, items: ranges.map((b) => ({ from: b.from!, to: b.to!, label: b.note || b.label, kind: 'bookmark' })) });
    }
    return this.added(
      plural(items.length, {
        one: $localize`:@@share.reportActions.addedBookmark:Added ${items.length}:count: bookmark to the report`,
        other: $localize`:@@share.reportActions.addedBookmarks:Added ${items.length}:count: bookmarks to the report`,
      }),
    );
  }

  /** A chart: its definition and an SVG snapshot of what is drawn. */
  async addChart(def: ChartDef, option: EChartsCoreOption | null): Promise<boolean> {
    let svg: string | null;
    try {
      svg = option ? await this.visuals.chartSvg(option) : null;
    } catch {
      svg = null;
    }
    this.reports.add({ type: 'chart', caption: '', title: def.title, spec: def, svg });
    return this.added($localize`:@@share.reportActions.addedChart:Added the chart "${def.title}:title:" to the report`);
  }

  addTimeline(): boolean {
    const svg = this.visuals.timelineSvg();
    const t = this.store.timeline();
    if (!svg || !t) return false;
    this.reports.add({ type: 'timeline', caption: '', svg, from: t.from, to: t.to });
    return this.added($localize`:@@share.reportActions.addedTimeline:Added the timeline to the report`);
  }

  /** The anomalies of the current results, most severe first, as findings. */
  async addAnomalies(): Promise<boolean> {
    try {
      const found = await this.analysis.queryAnomalies(this.store.effectiveQuery());
      if (!found.length) {
        this.store.setNotice($localize`:@@share.reportActions.noAnomalies:No anomalies in the current results`);
        return false;
      }
      const tz = this.store.timezone();
      const fmt = (ms: number) => formatFull(ms, tz);
      for (const a of [...found].sort((x, y) => y.score - x.score).slice(0, MAX_FINDINGS)) {
        this.reports.add({ type: 'finding', severity: anomalySeverity(a), title: a.message, detail: a.instance ? $localize`:@@share.reportActions.anomalyOn:${a.kind}:kind: on ${a.instance}:instance:, ${fmt(a.from)}:from: – ${fmt(a.to)}:to:` : `${a.kind}, ${fmt(a.from)} – ${fmt(a.to)}` });
      }
      const n = Math.min(found.length, MAX_FINDINGS);
      return this.added(
        plural(n, {
          one: $localize`:@@share.reportActions.addedFinding:Added ${n}:count: finding to the report`,
          other: $localize`:@@share.reportActions.addedFindings:Added ${n}:count: findings to the report`,
        }),
      );
    } catch (e) {
      this.store.setError($localize`:@@share.reportActions.anomaliesFailed:Could not load the anomalies: ${errorMessage(e)}:error:`);
      return false;
    }
  }

  /** The health check's findings for the current results (most severe first), each as a finding block. */
  async addFindings(): Promise<boolean> {
    try {
      const found = await this.share().overview(this.store.effectiveQuery());
      if (!found.length) {
        this.store.setNotice($localize`:@@share.reportActions.noHealth:The health check found nothing to report`);
        return false;
      }
      for (const f of found.slice(0, MAX_FINDINGS)) this.reports.add({ type: 'finding', severity: f.severity, title: f.title, detail: f.detail });
      const n = Math.min(found.length, MAX_FINDINGS);
      return this.added(
        plural(n, {
          one: $localize`:@@share.reportActions.addedHealth:Added ${n}:count: health-check finding to the report`,
          other: $localize`:@@share.reportActions.addedHealthMany:Added ${n}:count: health-check findings to the report`,
        }),
      );
    } catch (e) {
      this.store.setError($localize`:@@share.reportActions.healthFailed:Could not run the health check: ${errorMessage(e)}:error:`);
      return false;
    }
  }

  /** The last query-language table result. */
  addQueryTable(): boolean {
    const t = this.session.table();
    if (!t) return false;
    this.reports.add({ type: 'table', caption: '', query: this.session.applied() || this.session.text(), columns: t.columns, rows: t.rows.slice(0, MAX_TABLE_ROWS) });
    return this.added($localize`:@@share.reportActions.addedQuery:Added the query result to the report`);
  }

  /** The annotations shown on the timeline. */
  addAnnotations(): boolean {
    const items = this.annotations.items().filter((a) => this.annotations.isShown(a.kind));
    if (!items.length) return false;
    this.reports.add({ type: 'annotations', caption: $localize`:@@share.reportActions.annotationsCaption:Annotations`, items: items.map((a) => ({ from: a.from, to: a.to, label: a.label, kind: a.kind })) });
    return this.added(
      plural(items.length, {
        one: $localize`:@@share.reportActions.addedAnnotation:Added ${items.length}:count: annotation to the report`,
        other: $localize`:@@share.reportActions.addedAnnotations:Added ${items.length}:count: annotations to the report`,
      }),
    );
  }
}
