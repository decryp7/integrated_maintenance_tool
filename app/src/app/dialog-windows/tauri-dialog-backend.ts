import { inject, Injectable } from '@angular/core';
import { Logger } from '../core/logger';
import { Rect } from '../dock/dock-geometry';
import { clampToWorkAreas } from '../popout/bounds';
import { errorMessage } from '../tools/tool-errors';
import { loadDialogBounds, rememberDialogBounds } from './dialog-bounds';
import { DialogLabels } from './dialog-labels';
import {
  DIALOG_CLOSED_EVENT,
  DIALOG_IO,
  DIALOG_RESULT_EVENT,
  DialogClosedEvent,
  DialogOpenRequest,
  DialogResultEvent,
} from './dialog-io';
import type { DialogPresenter } from './dialog-presenter';
import { InPageDialogBackend } from './in-page-dialog-backend';
import { ConfirmQuestion, DialogSession, DialogShowRequest } from './dialog-session';

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

/** A result event of the shell; null when malformed. */
export function parseResult(p: unknown): DialogResultEvent | null {
  return isRecord(p) && typeof p['label'] === 'string' && typeof p['dialogId'] === 'string' ? { label: p['label'], dialogId: p['dialogId'], result: p['result'] } : null;
}

/** A closed event of the shell; null when malformed. */
export function parseClosed(p: unknown): DialogClosedEvent | null {
  if (!isRecord(p) || typeof p['label'] !== 'string' || typeof p['dialogId'] !== 'string') return null;
  const b = p['bounds'];
  const bounds = isRecord(b) && ['x', 'y', 'width', 'height'].every((k) => typeof b[k] === 'number') ? (b as unknown as Rect) : null;
  return { label: p['label'], dialogId: p['dialogId'], bounds };
}

interface Pending {
  dialogId: string;
  resolve: (r: unknown) => void;
  result: unknown;
}

/**
 * Dialogs as real OS windows (ADR 0050): `dialog_open` asks the shell for a window owned by this one (centred on it,
 * modal or not, at the size it had last time), the dialog answers with an event, and the window is gone. When
 * a window cannot be opened the dialog is shown in the page instead (degrade, don't fail).
 */
@Injectable({ providedIn: 'root' })
export class TauriDialogBackend implements DialogPresenter {
  private readonly io = inject(DIALOG_IO);
  private readonly inPage = inject(InPageDialogBackend);
  private readonly labels = inject(DialogLabels);
  private readonly logger = inject(Logger);
  private readonly pending = new Map<string, Pending>();
  private listening: Promise<void> | null = null;
  readonly windowed = true;

  show(request: DialogShowRequest): DialogSession {
    const def = request.def;
    if (!def) return this.inPage.show(request);
    let resolve!: (r: unknown) => void;
    const closed = new Promise<unknown>((r) => (resolve = r));
    let label: string | null = null;
    let fallback: DialogSession | null = null;
    let sent: DialogOpenRequest | null = null;
    const wire = async (): Promise<DialogOpenRequest> => {
      const areas = await this.io.workAreas().catch(() => []);
      const remembered = loadDialogBounds()[def.id];
      return {
        dialogId: def.id,
        instance: request.instance,
        title: request.title,
        modality: def.modality,
        requiredChoice: def.requiredChoice,
        input: request.inputs,
        width: request.width,
        height: def.height,
        minWidth: def.minWidth,
        minHeight: def.minHeight,
        bounds: remembered ? clampToWorkAreas(remembered, areas) : undefined,
      };
    };
    const start = async () => {
      try {
        await this.listen();
        sent = await wire();
        const info = await this.io.open(sent);
        label = info.label;
        this.labels.add(info.label);
        this.pending.set(info.label, { dialogId: def.id, resolve, result: undefined });
      } catch (e) {
        this.logger.warn('ui.dialog', 'Dialog window could not open; shown in the page', { Dialog: def.id, Error: errorMessage(e) });
        fallback = this.inPage.show(request);
        void fallback.closed.then(resolve);
      }
    };
    const opened = start();
    return {
      closed,
      focus: () => void opened.then(() => (fallback ? fallback.focus() : sent ? this.io.open(sent).then(() => undefined, () => undefined) : undefined)),
      close: (result) => void opened.then(() => (fallback ? fallback.close(result) : label ? this.io.requestClose(label).catch(() => undefined) : undefined)),
    };
  }

  readonly confirm = (question: ConfirmQuestion): Promise<boolean> =>
    this.io.ask(question).catch((e: unknown) => {
      this.logger.warn('ui.dialog', 'The message box could not be shown; treated as No', { Error: errorMessage(e) });
      return false;
    });

  private listen(): Promise<void> {
    this.listening ??= Promise.all([
      this.io.listen(DIALOG_RESULT_EVENT, (p) => {
        const e = parseResult(p);
        const entry = e && this.pending.get(e.label);
        if (entry && e) entry.result = e.result ?? undefined;
      }),
      this.io.listen(DIALOG_CLOSED_EVENT, (p) => {
        const e = parseClosed(p);
        const entry = e && this.pending.get(e.label);
        if (!e || !entry) return;
        this.pending.delete(e.label);
        this.labels.remove(e.label);
        if (e.bounds) rememberDialogBounds(entry.dialogId, e.bounds);
        entry.resolve(entry.result === null ? undefined : entry.result);
      }),
    ]).then(() => undefined);
    return this.listening;
  }
}
