import { inject, Injectable } from '@angular/core';
import { WorkspaceStore } from '../../core/workspace.store';
import { DebriefCommand } from '../../extensions/tokens';
import { QuerySession } from '../../analysis/query-session';
import { NotesStore } from '../../notes/notes.store';
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
  private readonly notes = inject(NotesStore);
  private readonly session = inject(QuerySession);
  private readonly annotations = inject(TimelineAnnotations);
  private readonly dialogs = inject(DialogService);

  /** Runs an add action and shows the report when it added something. */
  private add(ctx: { showView(id: string): void }, done: boolean): void {
    if (done) ctx.showView('report');
  }

  readonly commands: DebriefCommand[] = [
    { id: 'report.show', title: $localize`:@@share.reportCommands.show:Show report builder`, category: $localize`:@@share.commands.categoryAnalyze:Analyze`, view: 'report', run: (ctx) => ctx.showView('report') },
    {
      id: 'report.new',
      title: $localize`:@@share.reportCommands.new:New incident report`,
      category: $localize`:@@share.commands.categoryAnalyze:Analyze`,
      run: (ctx) => {
        this.reports.create();
        ctx.showView('report');
      },
    },
    { id: 'report.addSelected', title: $localize`:@@share.reportCommands.addSelected:Add selected event to report`, category: $localize`:@@share.commands.categoryAnalyze:Analyze`, when: () => !!this.ws.selected(), run: (ctx) => this.add(ctx, this.actions.addSelectedEvent()) },
    { id: 'report.addNotes', title: $localize`:@@share.reportCommands.addNotes:Add notes to report`, category: $localize`:@@share.commands.categoryAnalyze:Analyze`, when: () => this.notes.items().length > 0, run: (ctx) => this.add(ctx, this.actions.addNotes()) },
    { id: 'report.addTimeline', title: $localize`:@@share.reportCommands.addTimeline:Add timeline to report`, category: $localize`:@@share.commands.categoryAnalyze:Analyze`, when: () => !!this.ws.timeline(), run: (ctx) => this.add(ctx, this.actions.addTimeline()) },
    { id: 'report.addFindings', title: $localize`:@@share.reportCommands.addFindings:Add health-check findings to report`, category: $localize`:@@share.commands.categoryAnalyze:Analyze`, when: () => !!this.ws.summary()?.total, run: async (ctx) => this.add(ctx, await this.actions.addFindings()) },
    { id: 'report.addAnomalies', title: $localize`:@@share.reportCommands.addAnomalies:Add anomalies to report as findings`, category: $localize`:@@share.commands.categoryAnalyze:Analyze`, when: () => !!this.ws.summary()?.total, run: async (ctx) => this.add(ctx, await this.actions.addAnomalies()) },
    { id: 'report.addQueryTable', title: $localize`:@@share.reportCommands.addQueryTable:Add query result to report`, category: $localize`:@@share.commands.categoryAnalyze:Analyze`, when: () => !!this.session.table(), run: (ctx) => this.add(ctx, this.actions.addQueryTable()) },
    { id: 'report.addAnnotations', title: $localize`:@@share.reportCommands.addAnnotations:Add annotations to report`, category: $localize`:@@share.commands.categoryAnalyze:Analyze`, when: () => this.annotations.items().length > 0, run: (ctx) => this.add(ctx, this.actions.addAnnotations()) },
    { id: 'report.export', title: $localize`:@@share.reportCommands.export:Export report…`, category: $localize`:@@share.commands.categoryFile:File`, when: () => !!this.reports.active(), run: async () => void (await openReportExport(this.dialogs)) },
  ];
}
