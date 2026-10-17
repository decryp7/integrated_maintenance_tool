import { inject, Injectable } from '@angular/core';
import { WorkspaceStore } from '../../core/workspace.store';
import { ImtCommand } from '../../extensions/tokens';
import { QuerySession } from '../../analysis/query-session';
import { BookmarkStore } from '../../projects/bookmarks/bookmark.store';
import { TimelineAnnotations } from '../../timeline/annotations.store';
import { DialogService } from '../../ui/dialog.service';
import { ReportActions } from './report-actions';
import { ReportStore } from './report.store';
import { openReportExport } from './report-export-dialog';

/** Commands for incident reports: show and create, "Add … to report" from across the app, export. */
@Injectable({ providedIn: 'root' })
export class ReportCommands {
  private readonly actions = inject(ReportActions);
  private readonly reports = inject(ReportStore);
  private readonly ws = inject(WorkspaceStore);
  private readonly bookmarks = inject(BookmarkStore);
  private readonly session = inject(QuerySession);
  private readonly annotations = inject(TimelineAnnotations);
  private readonly dialogs = inject(DialogService);

  /** Runs an add action and shows the report when it added something. */
  private add(ctx: { showView(id: string): void }, done: boolean): void {
    if (done) ctx.showView('report');
  }

  readonly commands: ImtCommand[] = [
    { id: 'report.show', title: 'Show report builder', category: 'Analyze', run: (ctx) => ctx.showView('report') },
    {
      id: 'report.new',
      title: 'New incident report',
      category: 'Analyze',
      run: (ctx) => {
        this.reports.create();
        ctx.showView('report');
      },
    },
    { id: 'report.addSelected', title: 'Add selected event to report', category: 'Analyze', when: () => !!this.ws.selected(), run: (ctx) => this.add(ctx, this.actions.addSelectedEvent()) },
    { id: 'report.addBookmarks', title: 'Add bookmarks to report', category: 'Analyze', when: () => this.bookmarks.items().length > 0, run: (ctx) => this.add(ctx, this.actions.addBookmarks()) },
    { id: 'report.addTimeline', title: 'Add timeline to report', category: 'Analyze', when: () => !!this.ws.timeline(), run: (ctx) => this.add(ctx, this.actions.addTimeline()) },
    { id: 'report.addFindings', title: 'Add health-check findings to report', category: 'Analyze', when: () => !!this.ws.summary()?.total, run: async (ctx) => this.add(ctx, await this.actions.addFindings()) },
    { id: 'report.addAnomalies', title: 'Add anomalies to report as findings', category: 'Analyze', when: () => !!this.ws.summary()?.total, run: async (ctx) => this.add(ctx, await this.actions.addAnomalies()) },
    { id: 'report.addQueryTable', title: 'Add query result to report', category: 'Analyze', when: () => !!this.session.table(), run: (ctx) => this.add(ctx, this.actions.addQueryTable()) },
    { id: 'report.addAnnotations', title: 'Add annotations to report', category: 'Analyze', when: () => this.annotations.items().length > 0, run: (ctx) => this.add(ctx, this.actions.addAnnotations()) },
    { id: 'report.export', title: 'Export report…', category: 'File', when: () => !!this.reports.active(), run: async () => void (await openReportExport(this.dialogs)) },
  ];
}
