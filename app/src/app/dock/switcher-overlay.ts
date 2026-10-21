import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { DockService } from './dock.service';
import { selected, selectId } from './switcher';

/**
 * The IDE navigator (Ctrl+Tab, VS style): the open tool windows and documents in most-recently-used order. Hold
 * Ctrl and press Tab to move, release Ctrl to switch; arrows, Enter, Escape and the mouse work too.
 */
@Component({
  selector: 'imt-dock-switcher',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (dock.switcher()) {
      <div class="imt-switcher-backdrop" data-imt-overlay (pointerdown)="dock.cancelSwitch()"></div>
      <div class="imt-switcher" role="dialog" aria-label="Switch to panel" i18n-aria-label="@@dock.switcher.label" (pointerdown)="$event.stopPropagation()">
        <div class="cols">
          @for (col of columns(); track col.title) {
            <div class="col">
              <h2 [id]="'imt-sw-' + col.kind">{{ col.title }}</h2>
              <ul role="listbox" [attr.aria-labelledby]="'imt-sw-' + col.kind" [attr.aria-activedescendant]="current()?.kind === col.kind ? activeId() : null">
                @for (item of col.items; track item.id) {
                  <li
                    role="option"
                    [id]="'imt-sw-item-' + item.id"
                    [class.on]="current()?.id === item.id"
                    [attr.aria-selected]="current()?.id === item.id"
                    (pointerenter)="pick(item.id)"
                    (click)="pick(item.id); dock.commitSwitch()"
                  >
                    {{ item.title }}
                  </li>
                } @empty {
                  <li class="none" role="presentation" i18n="@@dock.switcher.none">None</li>
                }
              </ul>
            </div>
          }
        </div>
        <p class="hint" i18n="@@dock.switcher.hint">Hold Ctrl and press Tab to move; release Ctrl to switch.</p>
      </div>
    }
  `,
  styles: `
    .imt-switcher-backdrop { position: fixed; inset: 0; z-index: 200; }
    .imt-switcher {
      position: fixed; z-index: 201; inset-inline-start: 50%; top: 22%; transform: translateX(-50%); min-width: 460px; max-width: 80vw;
      background: var(--tooltip-bg); color: var(--text); border: 1px solid var(--border-3); border-radius: 8px;
      box-shadow: var(--shadow); padding: 12px 14px;
    }
    .cols { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
    h2 { margin: 0 0 6px; font-size: var(--fs-xs); font-weight: 600; letter-spacing: 0.04em; text-transform: uppercase; color: var(--muted); }
    ul { list-style: none; margin: 0; padding: 0; max-height: 50vh; overflow: auto; }
    li { padding: 4px 8px; border-radius: 4px; font-size: var(--fs-md); cursor: pointer; }
    li.on { background: var(--accent); color: var(--accent-ink); }
    li.none { color: var(--faint); cursor: default; }
    .hint { margin: 10px 0 0; font-size: var(--fs-xs); color: var(--muted); }
  `,
})
export class DockSwitcher {
  protected readonly dock = inject(DockService);
  protected readonly current = computed(() => {
    const s = this.dock.switcher();
    return s ? selected(s) : null;
  });
  protected readonly activeId = computed(() => (this.current() ? `imt-sw-item-${this.current()!.id}` : null));
  protected readonly columns = computed(() => {
    const items = this.dock.switcher()?.items ?? [];
    return [
      { kind: 'tool', title: $localize`:@@dock.switcher.tools:Active tool windows`, items: items.filter((i) => i.kind === 'tool') },
      { kind: 'document', title: $localize`:@@dock.switcher.documents:Active documents`, items: items.filter((i) => i.kind === 'document') },
    ];
  });

  protected pick(id: string): void {
    const s = this.dock.switcher();
    if (s) this.dock.switcher.set(selectId(s, id));
  }
}
