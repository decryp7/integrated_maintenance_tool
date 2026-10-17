import { afterNextRender, ChangeDetectionStrategy, Component, effect, ElementRef, inject, Injector, untracked, viewChild } from '@angular/core';
import { ContextMenuService, MenuEntry } from './context-menu.service';

/** Accessible context menu: `role=menu`, arrow keys, Home/End, Escape (focus returns), click outside closes. */
@Component({
  selector: 'imt-context-menu-host',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (service.menu(); as m) {
      <div class="backdrop" (mousedown)="service.close()" (contextmenu)="$event.preventDefault(); service.close()"></div>
      <div #box class="menu popover" role="menu" aria-label="Actions" [style.left.px]="m.x" [style.top.px]="m.y" (keydown)="onKey($event)">
        @for (item of m.items; track item.label) {
          <button type="button" role="menuitem" class="item" tabindex="-1" [disabled]="item.disabled" (click)="run(item)">{{ item.label }}</button>
        }
      </div>
    }
  `,
  styles: `
    .backdrop { position: fixed; inset: 0; z-index: 220; }
    .menu { position: fixed; z-index: 221; min-width: 240px; padding: 4px; display: flex; flex-direction: column; }
    .item { height: 28px; display: flex; align-items: center; padding: 0 10px; border: 0; border-radius: 5px; background: transparent; color: var(--text-2); cursor: pointer; text-align: left; white-space: nowrap; }
    .item:hover:not(:disabled), .item:focus { background: var(--bg-hover); color: var(--text); outline: none; }
    .item:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
    .item:disabled { color: var(--faint); cursor: default; }
  `,
})
export class ContextMenuHost {
  protected readonly service = inject(ContextMenuService);
  private readonly injector = inject(Injector);
  private readonly box = viewChild<ElementRef<HTMLElement>>('box');

  constructor() {
    effect(() => {
      const menu = this.service.menu();
      if (!menu) return;
      untracked(() =>
        afterNextRender(
          () => {
            const el = this.box()?.nativeElement;
            if (!el) return;
            // Keep the menu inside the window.
            const r = el.getBoundingClientRect();
            if (r.right > window.innerWidth) el.style.left = `${Math.max(0, window.innerWidth - r.width - 8)}px`;
            if (r.bottom > window.innerHeight) el.style.top = `${Math.max(0, window.innerHeight - r.height - 8)}px`;
            this.items()[0]?.focus();
          },
          { injector: this.injector },
        ),
      );
    });
  }

  private items(): HTMLButtonElement[] {
    return [...(this.box()?.nativeElement.querySelectorAll<HTMLButtonElement>('[role=menuitem]:not(:disabled)') ?? [])];
  }

  protected run(item: MenuEntry): void {
    this.service.close(false);
    item.run();
  }

  protected onKey(e: KeyboardEvent): void {
    const items = this.items();
    const index = items.indexOf(document.activeElement as HTMLButtonElement);
    const focus = (i: number) => items[(i + items.length) % items.length]?.focus();
    switch (e.key) {
      case 'ArrowDown':
        focus(index + 1);
        break;
      case 'ArrowUp':
        focus(index < 0 ? -1 : index - 1);
        break;
      case 'Home':
        focus(0);
        break;
      case 'End':
        focus(-1);
        break;
      case 'Escape':
        this.service.close();
        break;
      case 'Tab':
        this.service.close();
        break;
      default:
        return;
    }
    e.preventDefault();
    e.stopPropagation();
  }
}
