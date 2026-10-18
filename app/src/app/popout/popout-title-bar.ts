import { ChangeDetectionStrategy, Component, DestroyRef, inject, input, output, signal } from '@angular/core';
import { PopoutBackend, ResizeDirection } from './popout-backend';
import { POPOUT_ROUTE } from './popout-route';
import { UiSync } from './ui-sync';

const EDGES: readonly { dir: ResizeDirection; cls: string }[] = [
  { dir: 'North', cls: 'n' },
  { dir: 'South', cls: 's' },
  { dir: 'West', cls: 'w' },
  { dir: 'East', cls: 'e' },
  { dir: 'NorthWest', cls: 'nw' },
  { dir: 'NorthEast', cls: 'ne' },
  { dir: 'SouthWest', cls: 'sw' },
  { dir: 'SouthEast', cls: 'se' },
];

/**
 * A pop-out window's own compact title bar (VS / VS Code tool window style): drag it to move the window (and drop
 * it on a guide of the main window to dock it), double-click to maximise or restore, Dock back, Always on top.
 * On macOS the native traffic lights sit at its left (overlay title bar); on Windows/Linux the window is frameless
 * and this bar draws minimise / maximise / close and the resize edges. It dims while the window is inactive.
 */
@Component({
  selector: 'imt-popout-title-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[class.inactive]': '!active()', '[class.mac]': "backend.chrome === 'overlay'" },
  template: `
    <header class="bar" (pointerdown)="dragStart($event)" (dblclick)="dblclick($event)">
      <span class="grip" aria-hidden="true" title="Drag to dock: drop on a guide in the main window" i18n-title="@@popout.titleBar.grip">⠿</span>
      <div class="titles">
        <h1 class="title">{{ title() }}</h1>
        <span class="host">{{ host() || 'IMT' }}</span>
      </div>
      <button type="button" class="tb-btn icon" (click)="dockBack.emit()" aria-label="Dock back into the main window" title="Dock back into the main window" i18n-aria-label="@@popout.titleBar.dockBack" i18n-title="@@popout.titleBar.dockBackTitle">
        <svg width="14" height="14" viewBox="0 0 24 24" aria-hidden="true"><path d="M4 5h16v14H4zM4 10h16M12 13v4M10 15l2 2 2-2" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" /></svg>
      </button>
      @if (backend.supportsAlwaysOnTop) {
        <button type="button" class="tb-btn icon" [class.on]="onTop()" [attr.aria-pressed]="onTop()" [attr.aria-label]="alwaysOnTopLabel" [title]="onTop() ? alwaysOnTopOnTitle : alwaysOnTopLabel" (click)="toggleOnTop.emit()">
          <svg width="14" height="14" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 17v5M8 3h8l-1 7 3 3H6l3-3z" [attr.fill]="onTop() ? 'currentColor' : 'none'" stroke="currentColor" stroke-width="2" stroke-linejoin="round" /></svg>
        </button>
      }
      @if (backend.chrome === 'frameless') {
        <div class="controls" role="group" aria-label="Window" i18n-aria-label="@@popout.titleBar.windowControls">
          <button type="button" class="tb-btn ctl" aria-label="Minimise" title="Minimise" i18n-aria-label="@@popout.titleBar.minimise" i18n-title="@@popout.titleBar.minimiseTitle" (click)="backend.minimize()">
            <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M1 5.5h8" stroke="currentColor" /></svg>
          </button>
          <button type="button" class="tb-btn ctl" [attr.aria-label]="maximized() ? restoreLabel : maximiseLabel" [title]="maximized() ? restoreLabel : maximiseLabel" (click)="toggleMaximize()">
            @if (maximized()) {
              <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M2.5 2.5V1h6.5v6.5H7.5M1 2.5h6.5V9H1z" fill="none" stroke="currentColor" /></svg>
            } @else {
              <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M1 1h8v8H1z" fill="none" stroke="currentColor" /></svg>
            }
          </button>
          <button type="button" class="tb-btn ctl close" aria-label="Close (dock back)" title="Close (the panel docks back)" i18n-aria-label="@@popout.titleBar.close" i18n-title="@@popout.titleBar.closeTitle" (click)="backend.closeWindow()">
            <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M1 1l8 8M9 1L1 9" stroke="currentColor" /></svg>
          </button>
        </div>
      }
    </header>
    @if (backend.chrome === 'frameless' && !maximized()) {
      @for (e of edges; track e.dir) {
        <div class="edge {{ e.cls }}" aria-hidden="true" (pointerdown)="resize($event, e.dir)"></div>
      }
    }
  `,
  styles: `
    :host { display: block; flex: none; }
    .bar {
      display: flex; align-items: center; gap: 2px; height: 30px; padding: 0 0 0 8px; user-select: none;
      background: var(--bg-0); color: var(--text); border-bottom: 1px solid var(--border); box-shadow: inset 0 2px 0 var(--accent);
    }
    :host(.mac) .bar { padding-inline-start: 78px; }
    :host(.inactive) .bar { box-shadow: none; color: var(--muted); }
    :host(.inactive) .title { color: var(--muted); }
    .grip { color: var(--faint); cursor: grab; font-size: 12px; }
    .titles { flex: 1; min-width: 0; display: flex; align-items: baseline; gap: 10px; cursor: default; }
    .title { margin: 0; font-size: 12px; font-weight: 600; white-space: nowrap; }
    .host { font-size: 11px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .tb-btn {
      height: 28px; border: 0; border-radius: 4px; background: transparent; color: var(--muted);
      cursor: pointer; display: inline-flex; align-items: center; justify-content: center;
    }
    .tb-btn.icon { width: 28px; }
    .tb-btn.icon.on { color: var(--accent); }
    .tb-btn:hover { color: var(--text); }
    .tb-btn:hover { background: var(--bg-hover); }
    .tb-btn:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
    .controls { display: flex; height: 100%; margin-inline-start: 4px; }
    .tb-btn.ctl { width: 40px; height: 100%; border-radius: 0; color: inherit; }
    .tb-btn.close:hover { background: #c42b1c; color: #fff; }
    .edge { position: fixed; z-index: 1000; }
    .edge.n { top: 0; left: 6px; right: 6px; height: 4px; cursor: ns-resize; }
    .edge.s { bottom: 0; left: 6px; right: 6px; height: 4px; cursor: ns-resize; }
    .edge.w { left: 0; top: 6px; bottom: 6px; width: 4px; cursor: ew-resize; }
    .edge.e { right: 0; top: 6px; bottom: 6px; width: 4px; cursor: ew-resize; }
    .edge.nw { top: 0; left: 0; width: 6px; height: 6px; cursor: nwse-resize; }
    .edge.ne { top: 0; right: 0; width: 6px; height: 6px; cursor: nesw-resize; }
    .edge.sw { bottom: 0; left: 0; width: 6px; height: 6px; cursor: nesw-resize; }
    .edge.se { bottom: 0; right: 0; width: 6px; height: 6px; cursor: nwse-resize; }
  `,
})
export class PopoutTitleBar {
  protected readonly backend = inject(PopoutBackend);
  private readonly sync = inject(UiSync);
  private readonly route = inject(POPOUT_ROUTE);
  readonly title = input.required<string>();
  readonly host = input('');
  readonly onTop = input(false);
  readonly dockBack = output<void>();
  readonly toggleOnTop = output<void>();
  protected readonly edges = EDGES;
  protected readonly alwaysOnTopLabel = $localize`:@@popout.titleBar.alwaysOnTop:Always on top`;
  protected readonly alwaysOnTopOnTitle = $localize`:@@popout.titleBar.alwaysOnTopOn:Always on top (on): click to stop`;
  protected readonly restoreLabel = $localize`:@@popout.titleBar.restore:Restore`;
  protected readonly maximiseLabel = $localize`:@@popout.titleBar.maximise:Maximise`;
  protected readonly maximized = signal(false);
  protected readonly active = signal(typeof document === 'undefined' || document.hasFocus());

  constructor() {
    const destroy = inject(DestroyRef);
    const focus = () => this.active.set(true);
    const blur = () => this.active.set(false);
    window.addEventListener('focus', focus);
    window.addEventListener('blur', blur);
    const refresh = () => void this.backend.isMaximized().then((m) => this.maximized.set(m), () => undefined);
    refresh();
    let off: (() => void) | null = null;
    void this.backend.onResized(refresh).then((u) => (off = u), () => undefined);
    destroy.onDestroy(() => {
      window.removeEventListener('focus', focus);
      window.removeEventListener('blur', blur);
      off?.();
    });
  }

  protected async toggleMaximize(): Promise<void> {
    await this.backend.toggleMaximize();
    this.maximized.set(await this.backend.isMaximized());
  }

  /** Double-clicking the bar (not a button) maximises or restores, as on a native title bar. */
  protected dblclick(e: MouseEvent): void {
    if ((e.target as Element).closest('button') || this.backend.chrome === 'browser') return;
    void this.toggleMaximize();
  }

  protected resize(e: PointerEvent, direction: ResizeDirection): void {
    if (e.button !== 0) return;
    e.preventDefault();
    void this.backend.startResize(direction);
  }

  /**
   * Pressing the bar (not a button) moves the window: the OS moves it in the app (and the main window shows its
   * dock guides while it is over it; resting on one docks the panel). A browser window cannot be moved by the page,
   * so there the pointer's screen position is sent to the main window instead (same guides, same docking).
   */
  protected async dragStart(e: PointerEvent): Promise<void> {
    if (e.button !== 0 || e.detail > 1 || (e.target as Element).closest('button')) return;
    const label = this.route?.label ?? '';
    if (this.backend.chrome !== 'browser') {
      e.preventDefault();
      if (await this.backend.startWindowDrag().catch(() => false)) this.sync.runInHost('dragEnd', label);
      return;
    }
    const el = e.currentTarget as HTMLElement;
    try {
      el.setPointerCapture(e.pointerId);
    } catch {
      /* synthetic events have no capturable pointer */
    }
    const move = (m: PointerEvent) => this.sync.runInHost('dragOver', JSON.stringify({ label, x: m.screenX, y: m.screenY }));
    const up = () => {
      el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerup', up);
      el.removeEventListener('pointercancel', up);
      this.sync.runInHost('dragEnd', label);
    };
    el.addEventListener('pointermove', move);
    el.addEventListener('pointerup', up);
    el.addEventListener('pointercancel', up);
  }
}
