import { inject, Injectable, Injector, signal } from '@angular/core';
import { DialogRef, OpenDialog } from '../ui/dialog.service';
import type { DialogPresenter } from './dialog-presenter';
import { ConfirmQuestion, DialogSession, DialogShowRequest } from './dialog-session';
import { DialogComponent } from './dialog-def';

let nextId = 1;

/**
 * Shows dialogs in the page: the browser build (dev server, e2e) and overlays that are not windows (the command
 * palette). `debrief-dialog-host` renders the stack: a modal dialog gets a blocking backdrop, a modeless one is a
 * floating panel that leaves the page usable.
 */
@Injectable({ providedIn: 'root' })
export class InPageDialogBackend implements DialogPresenter {
  private readonly injector = inject(Injector);
  readonly stack = signal<readonly OpenDialog[]>([]);
  readonly windowed = false;
  readonly confirm: ((q: ConfirmQuestion) => Promise<boolean>) | undefined = undefined;

  show(request: DialogShowRequest): DialogSession {
    const id = nextId++;
    let resolve!: (r: unknown) => void;
    const closed = new Promise<unknown>((r) => (resolve = r));
    const canClose = () => request.canClose?.() !== false && (request.component as unknown as DialogComponent).dialogCanClose?.(this.injector) !== false;
    const close = (result?: unknown) => {
      if (!this.stack().some((d) => d.id === id)) return;
      if (!canClose()) return;
      this.stack.update((s) => s.filter((d) => d.id !== id));
      resolve(result);
    };
    const ref: DialogRef = { closed, close };
    const options = { title: request.title, inputs: request.inputs, width: request.width };
    this.stack.update((s) => [...s, { id, component: request.component, options, ref, modality: request.def?.modality ?? 'modal' }]);
    return { closed, close, isOpen: () => this.stack().some((d) => d.id === id), focus: () => this.stack.update((s) => [...s.filter((d) => d.id !== id), ...s.filter((d) => d.id === id)]) };
  }
}
