import { afterNextRender, ChangeDetectionStrategy, Component, effect, ElementRef, inject, Injector, untracked, viewChild } from '@angular/core';
import { ContextMenuService, MenuEntry } from './context-menu.service';
import { formatKeybinding, isMac } from '../extensions/keybinding';

/** Accessible context menu: `role=menu`, arrow keys, Home/End, Escape (focus returns), click outside closes. */
@Component({
  selector: 'debrief-context-menu-host',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (service.menu(); as m) {
      <div class="backdrop" data-debrief-overlay (mousedown)="service.close()" (contextmenu)="$event.preventDefault(); service.close()"></div>
      <div #box class="menu popover" role="menu" aria-label="Actions" i18n-aria-label="@@ui.contextMenu.actions" [style.left.px]="m.x" [style.top.px]="m.y" (keydown)="onKey($event)">
        @for (item of m.items; track item.label; let i = $index) {
          @if (i > 0 && item.group !== m.items[i - 1]?.group) {
            <div class="sep" role="separator"></div>
          }
          <button type="button" role="menuitem" class="item" [class.hinted]="!!item.hint" tabindex="-1" [disabled]="item.disabled" [attr.aria-keyshortcuts]="item.shortcut ? ariaKeys(item.shortcut) : null" (click)="run(item)">
            <span class="row"><span>{{ item.label }}</span>@if (item.shortcut) {<kbd class="kbd">{{ keys(item.shortcut) }}</kbd>}</span>
            @if (item.hint) {
              <span class="hint">{{ item.hint }}</span>
            }
          </button>
        }
      </div>
    }
  `,
  styles: `
    .backdrop { position: fixed; inset: 0; z-index: 220; }
    .menu { position: fixed; z-index: 221; min-width: 240px; padding: 4px; display: flex; flex-direction: column; }
    .item { height: 28px; display: flex; align-items: center; padding: 0 10px; border: 0; border-radius: 5px; background: transparent; color: var(--text-2); cursor: pointer; text-align: start; white-space: nowrap; }
    .item:hover:not(:disabled), .item:focus { background: var(--bg-hover); color: var(--text); outline: none; }
    .item:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
    .item:disabled { color: var(--faint); cursor: default; }
    .item.hinted { height: auto; min-height: 28px; padding: 4px 10px; flex-direction: column; align-items: flex-start; justify-content: center; white-space: normal; max-width: 360px; }
    .hint { font-size: var(--fs-xs); color: var(--faint); }
    .row { display: flex; align-items: center; gap: 24px; width: 100%; }
    .row > :first-child { flex: 1; }
    .kbd { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--muted); }
    .sep { height: 1px; margin: 4px 6px; background: var(--border); }
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

  protected keys(binding: string): string {
    return formatKeybinding(binding);
  }

  /** The binding in `aria-keyshortcuts` syntax ("Control+C"). */
  protected ariaKeys(binding: string): string {
    return binding.split('+').map((p) => ({ mod: isMac() ? 'Meta' : 'Control', ctrl: 'Control', shift: 'Shift', alt: 'Alt' })[p.toLowerCase()] ?? p.toUpperCase()).join('+');
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
