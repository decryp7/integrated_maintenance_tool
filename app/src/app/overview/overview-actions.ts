import { inject, Injectable } from '@angular/core';
import { Finding } from '../core/api.types';
import { AnalysisBackend } from '../core/analysis-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { FormatWizard } from '../format/format-wizard';
import { DialogService } from '../ui/dialog.service';
import { ActionId, panelTarget, zoomWindow } from './overview-model';

/** Carries out a finding's action through the app's usual paths (query, command context, dialogs). */
@Injectable({ providedIn: 'root' })
export class OverviewActions {
  private readonly ctx = inject(COMMAND_CONTEXT);
  private readonly store = inject(WorkspaceStore);
  private readonly analysis = inject(AnalysisBackend);
  private readonly dialogs = inject(DialogService);

  /** Runs one button of a finding. Failures are shown to the user, never thrown. */
  async run(finding: Finding, id: ActionId): Promise<void> {
    const action = finding.action;
    if (!action) return;
    try {
      switch (id) {
        case 'jump':
        case 'showRange':
          if (action.kind !== 'zoom') return;
          if (id === 'jump') {
            const w = zoomWindow(action.from, action.to);
            this.ctx.setQuery({ from: w.from, to: w.to });
            if (action.recordId !== null) await this.ctx.jumpTo(action.recordId);
          } else {
            this.ctx.setQuery({ from: action.from, to: action.to });
            this.ctx.showView('events');
          }
          return;
        case 'showException':
          if (action.kind === 'exception') await this.showException(action.fingerprint, action.recordId);
          return;
        case 'openExceptions':
          this.ctx.showView('exceptions');
          return;
        case 'setupFormat':
          if (action.kind !== 'setupFormat') return;
          this.dialogs.open(FormatWizard, { title: $localize`:@@overview.actions.setupFormat:Set up format`, inputs: { path: action.path, name: action.path.split(/[\\/]/).pop() ?? action.path }, width: 760 });
          return;
        case 'openPanel':
          if (action.kind === 'openPanel') this.ctx.showView(panelTarget(action.panel).view);
          return;
      }
    } catch (e) {
      this.ctx.notifyError(`${finding.title}: ${e instanceof Error ? e.message : String(e)}`);
    }
  }

  /** Filters to the events of an exception group (the engine identifies the group by fingerprint). */
  private async showException(fingerprint: string, recordId: number | null): Promise<void> {
    const { result: groups } = await this.analysis.queryExceptions(this.store.effectiveQuery());
    const group = groups.find((g) => g.fingerprint === fingerprint);
    if (!group) {
      this.ctx.notifyError($localize`:@@overview.actions.exceptionGone:That exception is not in the current view any more. Clear the filters and try again.`);
      return;
    }
    this.ctx.addFilter({ field: '@exception', op: 'contains', value: group.typeName });
    this.ctx.showView('events');
    const id = recordId ?? group.sampleIds[0];
    if (id !== undefined) await this.ctx.jumpTo(id);
  }
}
