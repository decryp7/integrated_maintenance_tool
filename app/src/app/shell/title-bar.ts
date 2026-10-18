import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { Icon } from '../shared/icon';
import { CommandRegistry } from '../extensions/command-registry';
import { PaletteService } from '../palette/palette.service';
import { TitleExtras } from '../lifecycle/title-extras';
import { MenuBar } from './menu/menu-bar';
import { MENU_MODE } from './menu/app-menu.service';
import { NativeMenuBridge } from './menu/native-menu';
import { localizeDecimal } from '../i18n/locale';

@Component({
  selector: 'imt-title-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, MenuBar, TitleExtras],
  template: `
    <div class="brand">
      <span class="logo" aria-hidden="true">
        <svg width="12" height="12" viewBox="0 0 12 12"><path d="M1 9 L4 5 L7 7 L11 2" stroke="currentColor" stroke-width="1.8" fill="none" stroke-linecap="round" stroke-linejoin="round" /></svg>
      </span>
      <span class="name">IMT</span>
    </div>
    @if (mode === 'in-app') {
      <imt-menu-bar class="menus" />
    }
    <div class="palette-wrap">
      <button class="palette" (click)="openPalette()" title="Command palette (Ctrl/⌘+K)" i18n-title="@@shell.titleBar.paletteTitle">
        <imt-icon name="search" [size]="13" />
        <span class="ellipsis" i18n="@@shell.titleBar.palettePrompt">{{ store.summary()?.profileName ?? 'IMT' }} — type a command or press Ctrl/⌘+K</span>
        <span class="mono faint kbd">⌘K</span>
      </button>
    </div>
    <div class="state">
      @if (store.busy(); as busy) {
        <span class="spinner" aria-hidden="true"></span><span>{{ busy }}</span>
      } @else if (store.summary(); as s) {
        <span class="dot" style="background: var(--mk-start)"></span>
        <span i18n="@@shell.titleBar.indexed">Indexed · {{ seconds(s.loadMs) }} s</span>
      }
    </div>
    <imt-title-extras />
  `,
  host: { '(document:keydown)': 'onKey($event)' },
  styles: `
    :host { height: 36px; flex: none; display: flex; align-items: center; gap: 16px; padding: 0 12px; background: var(--bg-0); border-bottom: 1px solid var(--border); }
    .brand { display: flex; align-items: center; gap: 8px; }
    .logo { width: 20px; height: 20px; border-radius: 5px; background: var(--accent); color: var(--accent-ink); display: flex; align-items: center; justify-content: center; }
    .name { font-weight: 600; letter-spacing: 0.04em; }
    .palette-wrap { flex: 1; display: flex; justify-content: center; min-width: 0; }
    .palette { width: 440px; max-width: 100%; height: 24px; display: flex; align-items: center; gap: 8px; padding: 0 10px; border-radius: 6px; background: var(--bg-3); border: 1px solid var(--border-2); color: var(--faint); font-size: 12px; cursor: text; }
    .palette span:nth-child(2) { flex: 1; text-align: start; }
    .kbd { font-size: 11px; }
    .state { display: flex; align-items: center; gap: 8px; color: var(--faint); font-size: 12px; white-space: nowrap; }
    .spinner { width: 10px; height: 10px; border-radius: 50%; border: 2px solid var(--border-3); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
    @keyframes spin { to { transform: rotate(360deg); } }
  `,
})
export class TitleBar {
  /** Load time in seconds, one decimal, with the locale's separator. */
  protected seconds(ms: number): string {
    return localizeDecimal((ms / 1000).toFixed(1));
  }

  protected readonly store = inject(WorkspaceStore);
  private readonly registry = inject(CommandRegistry);
  private readonly palette = inject(PaletteService);
  /** On macOS the menus are in the native menu bar and the in-app bar is hidden. */
  protected readonly mode = inject(MENU_MODE);

  constructor() {
    if (this.mode === 'native') inject(NativeMenuBridge).start();
  }

  protected openPalette(): void {
    this.palette.open();
  }

  protected onKey(e: KeyboardEvent): void {
    const target = e.target as HTMLElement | null;
    const typing = !!target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable);
    if (e.key === '/' && !typing) {
      e.preventDefault();
      this.store.focusSearch();
      return;
    }
    // Inside a text field, Mod+Z / Mod+Shift+Z undo the typing (the browser's own undo), not the last edit.
    if (isTextEntry(target) && (e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === 'z') return;
    this.registry.runForKey(e);
  }
}

const TEXT_INPUTS = new Set(['', 'text', 'search', 'email', 'url', 'tel', 'password', 'number']);

/** Is the element a place where text is typed (so the browser's own undo applies)? */
export function isTextEntry(el: HTMLElement | null): boolean {
  if (!el) return false;
  if (el.tagName === 'TEXTAREA' || el.isContentEditable) return true;
  return el.tagName === 'INPUT' && TEXT_INPUTS.has((el.getAttribute('type') ?? '').toLowerCase());
}
