import { NgComponentOutlet } from '@angular/common';
import {
  afterNextRender,
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  effect,
  ElementRef,
  inject,
  Injector,
  signal,
  Type,
  untracked,
  viewChild,
} from '@angular/core';
import { Logger } from '../core/logger';
import { WorkspaceStore } from '../core/workspace.store';
import { PopoutBackend } from '../popout/popout-backend';

import { sharedUiState } from '../popout/shared-state';
import { UiSync } from '../popout/ui-sync';
import { errorMessage } from '../tools/tool-errors';
import { DIALOG_REF, DialogRef } from '../ui/dialog.service';
import { DialogComponent, DialogModality } from './dialog-def';
import { DIALOG_CLOSE_REQUESTED_EVENT, DIALOG_IO } from './dialog-io';
import { dialogKeyAction, findDefaultAction } from './dialog-keys';
import { dialogDef } from './dialog-registry';
import { DIALOG_ROUTE } from './dialog-route';

/** The dialog window starts syncing once its workspace loaded, or after this long anyway. */
const SYNC_START_TIMEOUT_MS = 5000;

/**
 * The small app a dialog window boots into (ADR 0050): the dialog's content under the OS title bar. It takes its input
 * from the shell, shares the host's UI state through `UiSync` like a pop-out, answers through `DIALOG_REF.close`
 * and closes itself on Escape / Cmd+W (asking the component first) and runs the default action on Enter.
 */
@Component({
  selector: 'debrief-dialog-shell',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [NgComponentOutlet],
  host: { '(document:keydown)': 'onKey($event)', '[attr.data-modality]': 'modality()' },
  template: `
    <main #body class="body" [attr.aria-label]="title()" [attr.aria-busy]="!component() && !error()">
      @if (error(); as e) {
        <p class="error" role="alert" data-testid="dialog-error">{{ e }}</p>
        <div class="foot"><button type="button" class="btn primary" (click)="close()" i18n="@@dialogWindow.close">Close</button></div>
      } @else if (component(); as c) {
        <ng-container *ngComponentOutlet="c; inputs: inputs(); injector: contentInjector" />
      } @else {
        <p class="loading" data-testid="dialog-loading" i18n="@@dialogWindow.loading">Loading…</p>
      }
    </main>
  `,
  styles: `
    :host { display: block; height: 100vh; background: var(--bg-2); color: var(--text); }
    .body { box-sizing: border-box; height: 100%; overflow: auto; padding: 16px; display: flex; flex-direction: column; gap: 12px; }
    .loading { margin: auto; color: var(--muted); font-size: var(--fs-md); }
    .error { color: var(--lvl-error); margin: 0; }
    .foot { display: flex; justify-content: flex-end; }
  `,
})
export class DialogShell {
  private readonly route = inject(DIALOG_ROUTE)!;
  private readonly io = inject(DIALOG_IO);
  private readonly injector = inject(Injector);
  private readonly logger = inject(Logger);
  private readonly sync = inject(UiSync);
  private readonly store = inject(WorkspaceStore);
  private readonly backend = inject(PopoutBackend);
  private readonly body = viewChild.required<ElementRef<HTMLElement>>('body');
  private closing = false;

  protected readonly title = signal('');
  protected readonly modality = signal<DialogModality>('modeless');
  protected readonly component = signal<Type<unknown> | null>(null);
  protected readonly inputs = signal<Record<string, unknown>>({});
  protected readonly error = signal<string | null>(null);

  /** What the content injects as `DIALOG_REF`: closing answers the host and ends this window. */
  readonly ref: DialogRef = { closed: new Promise(() => undefined), close: (result) => void this.close(result) };
  protected readonly contentInjector = Injector.create({ providers: [{ provide: DIALOG_REF, useValue: this.ref }], parent: this.injector });

  constructor() {
    const destroy = inject(DestroyRef);
    void this.init();
    let off: (() => void) | null = null;
    void this.io.listen(DIALOG_CLOSE_REQUESTED_EVENT, () => void this.close()).then((u) => (off = u), () => undefined);
    destroy.onDestroy(() => off?.());
    afterNextRender(() => this.body().nativeElement.focus());
    this.startSyncWhenLoaded(destroy);
  }

  private async init(): Promise<void> {
    try {
      const def = dialogDef(this.route.id);
      if (!def) throw new Error($localize`:@@dialogWindow.unknown:This dialog is not known to this version of Debrief: ${this.route.id}:id:`);
      const [start, component] = await Promise.all([this.io.take(), def.load()]);
      this.title.set(start.title);
      this.modality.set(start.modality);
      document.title = start.title;
      this.inputs.set(isRecord(start.input) ? start.input : {});
      this.component.set(component);
    } catch (e) {
      this.logger.error('ui.dialog', 'Dialog window failed to start', { Dialog: this.route.id, Error: errorMessage(e) });
      this.error.set(errorMessage(e));
    }
  }

  /** Closes this dialog with `result` (none: dismissed), unless the content refuses (unsaved edits). */
  protected async close(result?: unknown): Promise<void> {
    const c = this.component() as unknown as DialogComponent | null;
    if (this.closing || c?.dialogCanClose?.(this.contentInjector) === false) return;
    this.closing = true;
    try {
      await this.io.close(result);
    } catch (e) {
      this.closing = false;
      this.logger.error('ui.dialog', 'Dialog window could not close', { Dialog: this.route.id, Error: errorMessage(e) });
    }
  }

  protected onKey(e: KeyboardEvent): void {
    const action = dialogKeyAction(e);
    if (action === 'close') {
      e.preventDefault();
      void this.close();
    } else if (action === 'default') {
      const button = findDefaultAction(this.body().nativeElement);
      if (button) {
        e.preventDefault();
        button.click();
      }
    }
  }

  /** Like a pop-out: sync starts once this window loaded its workspace summary (or after a timeout). */
  private startSyncWhenLoaded(destroy: DestroyRef): void {
    let started = false;
    const start = () => {
      if (started) return;
      started = true;
      const port = this.backend.syncPort(this.route.host, false, () => [this.route.host]);
      this.sync.start(port, false, this.route.label, sharedUiState(this.injector));
    };
    const fallback = setTimeout(start, SYNC_START_TIMEOUT_MS);
    const ref = effect(
      () => {
        if (this.store.summary() && !this.store.busy()) untracked(start);
      },
      { injector: this.injector },
    );
    destroy.onDestroy(() => {
      clearTimeout(fallback);
      ref.destroy();
    });
  }
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}
