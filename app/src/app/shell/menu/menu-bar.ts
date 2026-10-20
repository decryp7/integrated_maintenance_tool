import { CdkMenu, CdkMenuBar, CdkMenuItem, CdkMenuItemCheckbox, CdkMenuTrigger } from '@angular/cdk/menu';
import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { ariaKeyshortcuts, formatKeybinding } from '../../extensions/keybinding';
import { AppMenu } from './app-menu.service';
import { MenuTriggerGuard } from './submenu-trigger';
import { MenuNode, TopMenu } from './menu-model';

/**
 * IMT's own menu bar (Windows, Linux and the browser build): the application menus as an ARIA menubar with real
 * submenus, built on `@angular/cdk/menu` (keyboard navigation, typeahead, focus handling, overlay placement that
 * flips near the window edges). The menus come from `AppMenu`, the model the macOS native menu bar is built from too.
 */
@Component({
  selector: 'imt-menu-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CdkMenuBar, CdkMenu, CdkMenuItem, CdkMenuItemCheckbox, CdkMenuTrigger, MenuTriggerGuard],
  template: `
    <div cdkMenuBar class="bar" aria-label="Application menu" i18n-aria-label="@@shell.menuBar.applicationMenu">
      @for (menu of app.menus(); track menu.id) {
        <button type="button" cdkMenuItem imtMenuTrigger class="top" [cdkMenuTriggerFor]="list" [cdkMenuTriggerData]="{ $implicit: [menu.id], label: menu.label }">{{ menu.label }}</button>
      }
    </div>

    <!-- Menus are looked up by path on every render: CDK keeps the data a menu was first opened with. -->
    <ng-template #list let-path let-label="label">
      <div cdkMenu class="menu" [attr.aria-label]="label" [attr.data-menu]="label">
        @for (node of itemsAt(path); track $index) {
          @switch (node.kind) {
            @case ('separator') {
              <div role="separator" class="sep"></div>
            }
            @case ('group') {
              <div role="group" class="group" [attr.aria-label]="node.label">
                <div class="group-label" aria-hidden="true">{{ node.label }}</div>
                @for (item of node.items; track item.id) {
                  @if (item.checked === undefined) {
                    <button type="button" cdkMenuItem class="item" [cdkMenuitemTypeaheadLabel]="item.label" (cdkMenuItemTriggered)="app.run(item.id)">
                      <span class="check" aria-hidden="true"></span><span class="label">{{ item.label }}</span><span class="hint" aria-hidden="true">{{ hint(item.keybinding) }}</span>
                    </button>
                  } @else {
                    <button type="button" cdkMenuItemCheckbox class="item" [cdkMenuItemChecked]="item.checked" [cdkMenuitemTypeaheadLabel]="item.label" (cdkMenuItemTriggered)="app.run(item.id)">
                      <span class="check" aria-hidden="true">{{ item.checked ? '✓' : '' }}</span><span class="label">{{ item.label }}</span><span class="hint" aria-hidden="true">{{ hint(item.keybinding) }}</span>
                    </button>
                  }
                }
              </div>
            }
            @case ('submenu') {
              <button
                type="button"
                cdkMenuItem
                imtMenuTrigger
                class="item"
                [cdkMenuitemTypeaheadLabel]="node.label"
                [cdkMenuTriggerFor]="list"
                [cdkMenuTriggerData]="{ $implicit: path.concat(node.id), label: node.label }"
              >
                <span class="check" aria-hidden="true"></span><span class="label">{{ node.label }}</span><span class="chevron" aria-hidden="true">›</span>
              </button>
            }
            @default {
              @if (node.checked === undefined) {
                <button
                  type="button"
                  cdkMenuItem
                  class="item"
                  [cdkMenuitemTypeaheadLabel]="node.label"
                  [attr.aria-keyshortcuts]="keys(node.keybinding)"
                  (cdkMenuItemTriggered)="app.run(node.id)"
                >
                  <span class="check" aria-hidden="true"></span><span class="label">{{ node.label }}</span><span class="hint" aria-hidden="true">{{ hint(node.keybinding) }}</span>
                </button>
              } @else {
                <button
                  type="button"
                  cdkMenuItemCheckbox
                  class="item"
                  [cdkMenuItemChecked]="node.checked"
                  [cdkMenuitemTypeaheadLabel]="node.label"
                  [attr.aria-keyshortcuts]="keys(node.keybinding)"
                  (cdkMenuItemTriggered)="app.run(node.id)"
                >
                  <span class="check" aria-hidden="true">{{ node.checked ? '✓' : '' }}</span><span class="label">{{ node.label }}</span><span class="hint" aria-hidden="true">{{ hint(node.keybinding) }}</span>
                </button>
              }
            }
          }
        }
      </div>
    </ng-template>
  `,
  styles: `
    :host { display: block; }
    .bar { display: flex; gap: 2px; }
    .top { height: 26px; padding: 0 8px; border: 0; border-radius: 5px; background: transparent; color: var(--muted); cursor: pointer; font: inherit; }
    .top:hover, .top:focus-visible, .top.cdk-menu-trigger[aria-expanded='true'] { background: var(--bg-hover); color: var(--text); outline: none; }
    .menu {
      min-width: 240px; max-width: min(420px, calc(100vw - 16px)); max-height: calc(100vh - 56px); overflow-y: auto; overscroll-behavior: contain;
      display: flex; flex-direction: column; padding: 4px; border-radius: 8px;
      background: var(--tooltip-bg); border: 1px solid var(--border-3); box-shadow: var(--shadow); color: var(--text-2); font-size: var(--fs-md);
    }
    .menu:focus { outline: none; }
    .item {
      display: flex; align-items: center; gap: 8px; flex: none; height: 26px; padding: 0 10px 0 6px; border: 0; border-radius: 5px;
      background: transparent; color: var(--text-2); cursor: pointer; text-align: start; font: inherit; white-space: nowrap;
    }
    .item:hover, .item:focus, .item[aria-expanded='true'] { background: var(--bg-active); color: var(--text); outline: none; }
    .item:focus-visible { box-shadow: inset 0 0 0 1px var(--accent); }
    .check { width: 14px; flex: none; text-align: center; color: var(--accent-text); }
    .label { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; }
    .hint { flex: none; margin-inline-start: 24px; color: var(--muted); font-family: var(--font-mono); font-size: var(--fs-xs); }
    .chevron { flex: none; margin-inline-start: 24px; color: var(--muted); }
    .sep { flex: none; height: 1px; margin: 4px 6px; background: var(--border-2); }
    .group { display: flex; flex-direction: column; }
    .group-label { padding: 6px 10px 2px 28px; font-size: var(--fs-xs); color: var(--muted); text-transform: uppercase; letter-spacing: 0.04em; }
  `,
})
export class MenuBar {
  protected readonly app = inject(AppMenu);

  /** The items of the menu at `path` (top menu id, then submenu ids) in the current menus; empty once it is gone. */
  protected itemsAt(path: readonly string[]): MenuNode[] {
    let items: readonly MenuNode[] = this.app.menus().find((m: TopMenu) => m.id === path[0])?.items ?? [];
    for (const id of path.slice(1)) {
      const sub = items.find((n) => n.kind === 'submenu' && n.id === id);
      items = sub?.kind === 'submenu' ? sub.items : [];
    }
    return items as MenuNode[];
  }

  protected hint(binding: string | undefined): string {
    return binding ? formatKeybinding(binding) : '';
  }

  /** `aria-keyshortcuts` value, e.g. "Meta+Shift+O" (Mod is Cmd on macOS, Ctrl elsewhere). */
  protected keys(binding: string | undefined): string | null {
    return binding ? ariaKeyshortcuts(binding) : null;
  }
}
