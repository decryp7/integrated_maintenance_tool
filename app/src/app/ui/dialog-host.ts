import { NgComponentOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, effect, ElementRef, inject, Injector, untracked, viewChildren } from '@angular/core';
import { Icon } from '../shared/icon';
import { DIALOG_REF, DialogService, OpenDialog } from './dialog.service';

const FOCUSABLE = 'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** Renders the dialog stack: backdrop, titled `role=dialog`, focus trap, Escape to close, focus restore. */
@Component({
  selector: 'debrief-dialog-host',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [NgComponentOutlet, Icon],
  // Some browsers (Safari) do not focus a clicked button, so Escape must also work from the document.
  host: { '(document:keydown.escape)': 'onEscape($event)' },
  template: `
    @for (d of dialogs.stack(); track d.id) {
      <div class="backdrop" data-debrief-overlay (mousedown)="d.ref.close()"></div>
      <div
        #box
        class="dialog"
        role="dialog"
        aria-modal="true"
        [attr.aria-labelledby]="'debrief-dialog-title-' + d.id"
        [style.width.px]="d.options.width ?? 560"
        (keydown)="onKey($event, box)"
      >
        <div class="head">
          <h2 [id]="'debrief-dialog-title-' + d.id">{{ d.options.title }}</h2>
          <button class="icon-btn" type="button" aria-label="Close dialog" i18n-aria-label="@@ui.dialog.closeAria" title="Close" i18n-title="@@ui.dialog.close" (click)="d.ref.close()"><debrief-icon name="close" [size]="14" /></button>
        </div>
        <div class="body">
          <ng-container *ngComponentOutlet="d.component; inputs: d.options.inputs ?? {}; injector: injectorFor(d)" />
        </div>
      </div>
    }
  `,
  styles: `
    .backdrop { position: fixed; inset: 0; z-index: 210; background: var(--scrim); }
    .dialog { position: fixed; z-index: 211; top: 56px; inset-inline-start: 50%; transform: translateX(-50%); max-width: calc(100vw - 32px); max-height: calc(100vh - 96px); display: flex; flex-direction: column; border-radius: 10px; background: var(--bg-2); border: 1px solid var(--border-3); box-shadow: var(--shadow); color: var(--text); }
    .head { flex: none; display: flex; align-items: center; justify-content: space-between; padding: 10px 12px 10px 16px; border-bottom: 1px solid var(--border); }
    h2 { margin: 0; font-size: var(--fs-lg); font-weight: 600; }
    .body { min-height: 0; overflow: auto; padding: 16px; display: flex; flex-direction: column; gap: 12px; }
  `,
})
export class DialogHost {
  protected readonly dialogs = inject(DialogService);
  private readonly parent = inject(Injector);
  private readonly boxes = viewChildren<ElementRef<HTMLElement>>('box');
  private readonly injectors = new Map<number, Injector>();
  private restore: HTMLElement | null = null;
  private depth = 0;

  constructor() {
    effect(() => {
      const count = this.dialogs.stack().length;
      const boxes = this.boxes();
      untracked(() => {
        if (count > this.depth) {
          if (this.depth === 0) this.restore = document.activeElement as HTMLElement | null;
          queueMicrotask(() => this.focusFirst(this.boxes().at(-1)?.nativeElement));
        } else if (count === 0 && this.depth > 0) {
          this.restore?.focus?.();
          this.restore = null;
        }
        this.depth = count;
        void boxes;
      });
    });
  }

  protected onEscape(e: Event): void {
    if (!this.dialogs.stack().length) return;
    e.preventDefault();
    this.dialogs.dismissTop();
  }

  protected injectorFor(d: OpenDialog): Injector {
    let inj = this.injectors.get(d.id);
    if (!inj) {
      inj = Injector.create({ providers: [{ provide: DIALOG_REF, useValue: d.ref }], parent: this.parent });
      this.injectors.set(d.id, inj);
    }
    return inj;
  }

  private focusFirst(box: HTMLElement | undefined): void {
    const target = box?.querySelector<HTMLElement>('.body ' + FOCUSABLE.split(', ').join(', .body ')) ?? box?.querySelector<HTMLElement>(FOCUSABLE);
    target?.focus();
  }

  protected onKey(e: KeyboardEvent, box: HTMLElement): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      this.dialogs.dismissTop();
      return;
    }
    if (e.key !== 'Tab') return;
    const items = [...box.querySelectorAll<HTMLElement>(FOCUSABLE)];
    const first = items[0];
    const last = items[items.length - 1];
    if (!first || !last) return;
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }
}
