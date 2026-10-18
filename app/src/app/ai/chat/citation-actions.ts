import { inject, Injectable } from '@angular/core';
import { QuerySession } from '../../analysis/query-session';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { ConfirmDialog } from '../../ui/confirm-dialog';
import { DialogService } from '../../ui/dialog.service';
import { AiBackend, toAiError } from '../ai-backend';

/** What the links in an answer do: jump to an event, apply a query, open a web page (after asking). */
@Injectable({ providedIn: 'root' })
export class AiCitations {
  private readonly ctx = inject(COMMAND_CONTEXT);
  private readonly session = inject(QuerySession);
  private readonly dialogs = inject(DialogService);
  private readonly backend = inject(AiBackend);

  /** Reveals the event in the grid (and the timeline follows the selection). */
  record(id: number): Promise<void> {
    return this.ctx.jumpTo(id);
  }

  /** Applies the query through the query bar; returns an error message when the engine refused it. */
  async query(ql: string): Promise<string | null> {
    this.session.load(ql);
    const result = await this.session.run();
    if (result === null) return this.session.runError() ?? $localize`:@@ai.citations.queryFailed:The query could not be run.`;
    this.ctx.showView(result === 'table' ? 'query-results' : 'query');
    return null;
  }

  /** Opens a cited web page in the system browser after the user confirms. */
  async web(url: string, title: string): Promise<string | null> {
    const ref = this.dialogs.open<boolean>(ConfirmDialog, {
      title: $localize`:@@ai.citations.openWeb.title:Open web page?`,
      width: 480,
      inputs: {
        message: $localize`:@@ai.citations.openWeb.message:Open "${title}:title:" in your web browser?`,
        detail: url,
        confirmLabel: $localize`:@@ai.citations.openWeb.confirm:Open in browser`,
      },
    });
    if ((await ref.closed) !== true) return null;
    try {
      await this.backend.openLink(url);
      return null;
    } catch (e) {
      return toAiError(e).message;
    }
  }
}
