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
  ViewContainerRef,
} from '@angular/core';
import { deniedCommand } from '../core/denied-command';
import { Logger } from '../core/logger';
import { REPORT_PROBLEM_COMMAND } from '../diagnostics/snapshot.types';
import { WorkspaceStore } from '../core/workspace.store';
import { PopoutBackend } from '../popout/popout-backend';

import { sharedUiState } from '../popout/shared-state';
import { UiSync } from '../popout/ui-sync';
import { errorMessage } from '../tools/tool-errors';
import { DIALOG_REF, DialogRef } from '../ui/dialog.service';
import { DialogComponent, DialogModality } from './dialog-def';
import { DIALOG_CLOSE_REQUESTED_EVENT, DIALOG_IO } from './dialog-io';
import { dialogKeyAction, findDefaultAction } from './dialog-keys';
import { DIALOG_LOOKUP } from './dialog-registry';
import { DIALOG_ROUTE } from './dialog-route';

/** The dialog window starts syncing once its workspace loaded, or after this long anyway. */
const SYNC_START_TIMEOUT_MS = 5000;
/** The shell body's padding (see the styles): the window is this much taller than its content. */
const BODY_PADDING_PX = 16;

/**
 * The small app a dialog window boots into (ADR 0050): the dialog's content under the OS title bar. It takes its input
 * from the shell, shares the host's UI state through `UiSync` like a pop-out, answers through `DIALOG_REF.close`
 * and closes itself on Escape / Cmd+W (asking the component first) and runs the default action on Enter.
 */
@Component({
  selector: 'debrief-dialog-shell',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '(document:keydown)': 'onKey($event)', '[attr.data-modality]': 'modality()' },
  template: `
    <main #body class="body" [attr.aria-label]="title()" [attr.aria-busy]="!component() && !error()">
      @if (error(); as e) {
        <div class="failed" role="alert" data-testid="dialog-error">
          <h1 class="failed-title" i18n="@@dialogWindow.failedTitle">This window couldn't open</h1>
          <p class="failed-reason" data-testid="dialog-error-reason">{{ e }}</p>
        </div>
        <div class="foot">
          <button type="button" class="btn" (click)="report()" data-testid="dialog-error-report" i18n="@@dialogWindow.report">Report a problem…</button>
          <button type="button" class="btn primary" (click)="close()" data-testid="dialog-error-close" i18n="@@dialogWindow.close">Close</button>
        </div>
      } @else if (!component()) {
        <p class="loading" data-testid="dialog-loading" i18n="@@dialogWindow.loading">Loading…</p>
      }
      @if (denied(); as d) {
        <p class="denied" role="alert" data-testid="dialog-denied">{{ d }}</p>
      }
      <ng-container #host />
    </main>
  `,
  styles: `
    /* A dialog window is a full-height column: the content fills it and scrolls inside (no sizes of the in-page overlay). */
    :host { --dialog-window-height: 100%; --dialog-window-max-height: none; display: block; height: 100vh; background: var(--bg-2); color: var(--text); }
    .body { box-sizing: border-box; height: 100%; min-height: 0; overflow: auto; padding: 16px; display: flex; flex-direction: column; gap: 12px; }
    .body > :not(.loading):not(.failed):not(.denied):not(.foot) { flex: 1 1 auto; min-height: 0; }
    .loading { margin: auto; color: var(--muted); font-size: var(--fs-md); }
    .failed { margin: auto 0; }
    .denied { margin: 0; padding: 8px 10px; border: 1px solid var(--lvl-error); border-radius: 6px; color: var(--lvl-error); }
    .failed-title { margin: 0 0 8px; font-size: var(--fs-lg); font-weight: 600; }
    .failed-reason { margin: 0; color: var(--lvl-error); overflow-wrap: anywhere; }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class DialogShell {
  private readonly route = inject(DIALOG_ROUTE)!;
  private readonly io = inject(DIALOG_IO);
  private readonly lookup = inject(DIALOG_LOOKUP);
  private readonly injector = inject(Injector);
  private readonly logger = inject(Logger);
  private readonly sync = inject(UiSync);
  private readonly store = inject(WorkspaceStore);
  private readonly backend = inject(PopoutBackend);
  private readonly body = viewChild.required<ElementRef<HTMLElement>>('body');
  private readonly host = viewChild.required('host', { read: ViewContainerRef });
  private closing = false;
  private mounted = false;

  protected readonly title = signal('');
  protected readonly modality = signal<DialogModality>('modeless');
  protected readonly component = signal<Type<unknown> | null>(null);
  protected readonly inputs = signal<Record<string, unknown>>({});
  protected readonly error = signal<string | null>(null);
  /** A command this window may not call was refused (the dialog's own error handling must not be the only place it shows). */
  protected readonly denied = deniedCommand;

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
    // The content is created here, not by an outlet, so a throw while it is constructed or first rendered becomes the error state instead of a blank window.
    effect(() => {
      const c = this.component();
      const host = this.host();
      if (c && !this.mounted) untracked(() => this.mount(host, c));
    });
  }

  private mount(host: ViewContainerRef, component: Type<unknown>): void {
    this.mounted = true;
    try {
      const ref = host.createComponent(component, { injector: this.contentInjector });
      for (const [name, value] of Object.entries(this.inputs())) ref.setInput(name, value);
      ref.changeDetectorRef.detectChanges();
    } catch (e) {
      host.clear();
      this.fail(e);
    }
  }

  private fail(e: unknown): void {
    this.logger.error('ui.dialog', 'Dialog window failed to start', { Dialog: this.route.id, Error: errorMessage(e) });
    this.component.set(null);
    this.error.set(errorMessage(e));
  }

  /** Asks the opener's window to open "Report a problem…" (the diagnostics belong to the main window). */
  protected report(): void {
    if (!this.sync.runInHost('dialogRun', REPORT_PROBLEM_COMMAND)) this.logger.warn('ui.dialog', 'Could not reach the main window to report', { Dialog: this.route.id });
  }

  private async init(): Promise<void> {
    try {
      const def = this.lookup(this.route.id);
      if (!def) throw new Error($localize`:@@dialogWindow.unknown:This dialog is not known to this version of Debrief: ${this.route.id}:id:`);
      const [start, component] = await Promise.all([this.io.take(), def.load()]);
      this.title.set(start.title);
      this.modality.set(start.modality);
      document.title = start.title;
      this.inputs.set(isRecord(start.input) ? start.input : {});
      this.component.set(component);
      if (def.height === undefined) this.fitToContent();
    } catch (e) {
      this.fail(e);
    }
  }

  /**
   * The window opens at a default size; unless the dialog names a height in the registry, it is sized to what its content
   * needs (once, after the first render: the user's own resizing afterwards stretches the content, which fills the window).
   */
  private fitToContent(): void {
    afterNextRender(
      () => {
        const content = this.body().nativeElement.firstElementChild;
        if (!content) return;
        // Measure at its natural height: the content normally stretches to fill the window.
        const el = content as HTMLElement;
        const flex = el.style.flex;
        el.style.flex = 'none';
        const h = el.getBoundingClientRect().height;
        el.style.flex = flex;
        if (h > 0) void this.io.fitHeight(h + 2 * BODY_PADDING_PX).catch(() => undefined);
      },
      { injector: this.injector },
    );
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
