import { inject, Injectable } from '@angular/core';
import { QuerySession } from '../../analysis/query-session';
import { ActivityLog } from '../../core/activity/activity-log';
import { isCancellation } from '../../core/activity/runtime';
import { MarkDialog } from '../../context/mark-dialog';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { ProfileUndo } from '../../undo/profile-undo';
import { WorkspaceStore } from '../../core/workspace.store';
import { ParsersBackend } from '../../settings/parsers/parsers-backend';
import { DialogService } from '../../ui/dialog.service';
import { Proposal } from '../ai.types';

/** What applying a proposal did: done (with a message), or handed over to a dialog the user finishes. */
export type ApplyOutcome = { kind: 'applied'; message: string } | { kind: 'opened'; message: string } | { kind: 'cancelled' };

const LABEL_MAX = 40;

/**
 * Applies an engine-validated proposal through the same paths a user's own edit takes: a parser for a file
 * into the library (then the files are read again, ADR 0026), profile changes through the profile's validated
 * save (undoable), a regex as a highlight rule in the marker dialog, a
 * query through the query bar. The AI never changes anything itself.
 */
@Injectable({ providedIn: 'root' })
export class ProposalApply {
  private readonly profiles = inject(ProfileUndo);
  private readonly dialogs = inject(DialogService);
  private readonly query = inject(QuerySession);
  private readonly ctx = inject(COMMAND_CONTEXT);
  private readonly activity = inject(ActivityLog);
  private readonly parsers = inject(ParsersBackend);
  private readonly workspace = inject(WorkspaceStore);

  /** Applies `p` as one user operation in the activity log (`ai.applyProposal`, with its kind only). */
  async apply(p: Proposal): Promise<ApplyOutcome> {
    const op = this.activity.runtime.begin('ai.applyProposal', 'auto', { Kind: p.kind });
    try {
      const outcome = await this.applyNow(p);
      this.activity.runtime.end(op, outcome.kind === 'cancelled' ? 'cancelled' : 'ok');
      return outcome;
    } catch (e) {
      this.activity.runtime.end(op, isCancellation(e) ? 'cancelled' : 'error', e);
      throw e;
    }
  }

  private async applyNow(p: Proposal): Promise<ApplyOutcome> {
    switch (p.kind) {
      case 'regex': {
        const label = p.rationale.trim() || $localize`:@@ai.proposalApply.defaultLabel:AI pattern`;
        const ref = this.dialogs.open(MarkDialog, {
          title: $localize`:@@ai.proposalApply.highlight.title:Highlight lines`,
          width: 760,
          inputs: {
            initial: [{ field: '@message', op: 'matches', value: p.pattern }],
            suggestedLabel: label.length > LABEL_MAX ? `${label.slice(0, LABEL_MAX).trim()}…` : label,
            submitLabel: $localize`:@@ai.proposalApply.highlight.submit:Add highlight`,
            hint: $localize`:@@ai.proposalApply.highlight.hint:The suggested regex as a highlight rule. Review it; the preview shows what it highlights.`,
          },
        });
        return (await ref.closed) ? { kind: 'applied', message: $localize`:@@ai.proposalApply.highlight.done:Highlight rule added.` } : { kind: 'cancelled' };
      }
      case 'query': {
        this.query.load(p.ql);
        const result = await this.query.run();
        if (result === null) throw new Error(this.query.runError() ?? $localize`:@@ai.proposalApply.queryFailed:The query could not be run.`);
        if (result === 'table') this.ctx.showView('query-results');
        return {
          kind: 'applied',
          message:
            result === 'table'
              ? $localize`:@@ai.proposalApply.query.table:Query run; see Query results.`
              : $localize`:@@ai.proposalApply.query.filter:Query applied as the filter.`,
        };
      }
      case 'parser':
        // A parser for the library (ADR 0026); none when the library already reads the file.
        if (!p.toml.trim()) return { kind: 'applied', message: $localize`:@@ai.proposalApply.parser.known:Debrief's parsers already read this file.` };
        await this.parsers.save(p.toml, { overwrite: true });
        void this.workspace.followReread(await this.parsers.reread());
        return { kind: 'applied', message: $localize`:@@ai.proposalApply.parser.done:Parser added to the library; the files are read again.` };
      case 'pipeline':
        await this.profiles.save($localize`:@@ai.proposalApply.pipeline.undo:AI: pipeline steps`, p.profileToml);
        return { kind: 'applied', message: $localize`:@@ai.proposalApply.pipeline.done:Pipeline steps added to the profile (Undo reverts them).` };
      case 'markers':
        await this.profiles.save($localize`:@@ai.proposalApply.markers.undo:AI: markers`, p.profileToml);
        return { kind: 'applied', message: $localize`:@@ai.proposalApply.markers.done:Markers added to the profile (Undo reverts them).` };
      case 'extractors':
        await this.profiles.save($localize`:@@ai.proposalApply.extractors.undo:AI: extractors`, p.profileToml);
        return { kind: 'applied', message: $localize`:@@ai.proposalApply.extractors.done:Extractors added to the profile (Undo reverts them).` };
    }
  }
}
