import {
  afterNextRender,
  ApplicationRef,
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  ElementRef,
  EnvironmentInjector,
  inject,
  OnDestroy,
  untracked,
  viewChild,
  ViewEncapsulation,
} from '@angular/core';
import { ViewRegistry } from '../extensions/view-registry';
import { AngularContentRenderer } from './angular-panel';
import { AutoHideItem, itemsOn } from './auto-hide';
import { Side } from './dock-geometry';
import { DockKeys } from './dock-keys';
import { DockService } from './dock.service';
import { DockSwitcher } from './switcher-overlay';

/** How long the pointer may be away from a slid-out (hover-opened) tool window before it hides again. */
export const HOVER_HIDE_MS = 300;

/**
 * Hosts the dockview instance managed by `DockService`, with Visual Studio's extras around it: the auto-hide side
 * bars and their slide-out, the Ctrl+Tab navigator, inline dock notices and the full-screen mode.
 */
@Component({
  selector: 'imt-dock-host',
  changeDetection: ChangeDetectionStrategy.OnPush,
  encapsulation: ViewEncapsulation.None,
  imports: [DockSwitcher],
  template: `
    @for (side of sides; track side) {
      @if (bars()[side].length) {
        <nav class="imt-ah-bar imt-ah-{{ side }}" [attr.aria-label]="barLabel(side)">
          @for (item of bars()[side]; track item.id) {
            <button
              type="button"
              class="imt-ah-tab"
              [class.open]="open()?.id === item.id"
              [attr.aria-expanded]="open()?.id === item.id"
              [attr.aria-controls]="'imt-ah-flyout'"
              [attr.data-ah-id]="item.id"
              (mouseenter)="hover(item.id)"
              (mouseleave)="leaveSoon()"
              (click)="click(item.id, $event)"
              (keydown.escape)="dock.autoHideEvent({ type: 'escape' })"
            >
              {{ item.title }}
            </button>
          }
        </nav>
      }
    }
    <div class="imt-dock-center">
      <div #mount class="imt-dock-mount"></div>
      @if (open(); as item) {
        <section
          id="imt-ah-flyout"
          class="imt-ah-flyout imt-ah-flyout-{{ item.side }}"
          role="region"
          tabindex="-1"
          [attr.aria-label]="item.title"
          [style.width.px]="item.side === 'bottom' ? null : item.size"
          [style.height.px]="item.side === 'bottom' ? item.size : null"
          (mouseenter)="cancelLeave()"
          (mouseleave)="leaveSoon()"
          (keydown.escape)="escape()"
          (focusout)="focusOut($event)"
        >
          <header class="imt-ah-head">
            <span class="imt-ah-title">{{ item.title }}</span>
            <button type="button" class="imt-dock-action" [attr.aria-label]="pinLabel(item.title)" title="Pin (dock it again)" i18n-title="@@dock.host.pinTitle" (click)="dock.pin(item.id)">
              <svg width="13" height="13" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 17v5M8 3h8l-1 7 3 3H6l3-3z" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round" /></svg>
            </button>
            <button type="button" class="imt-dock-action" [attr.aria-label]="hideLabel(item.title)" title="Hide (Esc)" i18n-title="@@dock.host.hideTitle" (click)="escape()">
              <svg width="13" height="13" viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>
            </button>
          </header>
          <div #flyoutBody class="imt-ah-body"></div>
          <div class="imt-ah-grip" aria-hidden="true" (pointerdown)="startResize($event, item)"></div>
        </section>
      }
      @if (dock.notice(); as notice) {
        <div class="imt-dock-notice" role="alert">
          <span>{{ notice.message }}</span>
          @if (notice.retry) {
            <button type="button" class="btn" (click)="retry()" i18n="@@dock.host.retry">Retry</button>
          }
          <button type="button" class="btn" (click)="dock.notice.set(null)" i18n="@@dock.host.dismiss">Dismiss</button>
        </div>
      }
    </div>
    @if (dock.switcher()) {
      <imt-dock-switcher />
    }
  `,
  styles: `
    imt-dock-host {
      display: grid; flex: 1; min-height: 0; min-width: 0; position: relative;
      grid-template-columns: auto minmax(0, 1fr) auto; grid-template-rows: minmax(0, 1fr) auto;
      grid-template-areas: 'left center right' 'bottom bottom bottom';
    }
    .imt-dock-center { grid-area: center; position: relative; min-width: 0; min-height: 0; display: flex; }
    .imt-dock-mount { flex: 1; min-width: 0; min-height: 0; position: relative; }

    /* Auto-hide side bars (VS): vertical labels on the sides, horizontal at the bottom. */
    .imt-ah-bar { display: flex; gap: 2px; padding: 2px; background: var(--bg-0); }
    .imt-ah-left { grid-area: left; flex-direction: column; border-right: 1px solid var(--border); }
    .imt-ah-right { grid-area: right; flex-direction: column; border-left: 1px solid var(--border); }
    .imt-ah-bottom { grid-area: bottom; flex-direction: row; border-top: 1px solid var(--border); }
    .imt-ah-tab {
      border: 0; background: transparent; color: var(--muted); font: inherit; font-size: 12px; cursor: pointer;
      padding: 8px 4px; border-radius: 3px; white-space: nowrap;
    }
    .imt-ah-left .imt-ah-tab, .imt-ah-right .imt-ah-tab { writing-mode: vertical-rl; }
    .imt-ah-left .imt-ah-tab { transform: rotate(180deg); }
    .imt-ah-bottom .imt-ah-tab { padding: 3px 10px; }
    .imt-ah-tab:hover, .imt-ah-tab.open { color: var(--text); background: var(--bg-hover); box-shadow: inset 0 -2px 0 var(--accent); }
    .imt-ah-left .imt-ah-tab:hover, .imt-ah-left .imt-ah-tab.open { box-shadow: inset 2px 0 0 var(--accent); }
    .imt-ah-right .imt-ah-tab:hover, .imt-ah-right .imt-ah-tab.open { box-shadow: inset -2px 0 0 var(--accent); }
    .imt-ah-tab:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }

    .imt-ah-flyout {
      position: absolute; z-index: 990; display: flex; flex-direction: column; background: var(--bg-1);
      border: 1px solid var(--border-3); box-shadow: var(--shadow); outline: none;
    }
    .imt-ah-flyout-left { left: 0; top: 0; bottom: 0; animation: imt-slide-left 140ms ease-out; }
    .imt-ah-flyout-right { right: 0; top: 0; bottom: 0; animation: imt-slide-right 140ms ease-out; }
    .imt-ah-flyout-bottom { left: 0; right: 0; bottom: 0; animation: imt-slide-up 140ms ease-out; }
    @keyframes imt-slide-left { from { transform: translateX(-24px); opacity: 0.4; } }
    @keyframes imt-slide-right { from { transform: translateX(24px); opacity: 0.4; } }
    @keyframes imt-slide-up { from { transform: translateY(24px); opacity: 0.4; } }
    @media (prefers-reduced-motion: reduce) { .imt-ah-flyout { animation: none; } }
    .imt-ah-head {
      display: flex; align-items: center; gap: 2px; height: 30px; padding: 0 4px 0 10px; flex: none;
      background: var(--bg-0); border-bottom: 1px solid var(--border); box-shadow: inset 0 2px 0 var(--accent);
    }
    .imt-ah-title { flex: 1; font-size: 12px; color: var(--text); }
    .imt-ah-body { flex: 1; min-height: 0; display: flex; }
    .imt-ah-body > .imt-panel { flex: 1; }
    .imt-ah-grip { position: absolute; background: transparent; }
    .imt-ah-flyout-left .imt-ah-grip { right: -3px; top: 0; bottom: 0; width: 6px; cursor: ew-resize; }
    .imt-ah-flyout-right .imt-ah-grip { left: -3px; top: 0; bottom: 0; width: 6px; cursor: ew-resize; }
    .imt-ah-flyout-bottom .imt-ah-grip { top: -3px; left: 0; right: 0; height: 6px; cursor: ns-resize; }

    .imt-dock-notice {
      position: absolute; z-index: 995; inset-inline-start: 50%; top: 8px; transform: translateX(-50%); max-width: 80%;
      display: flex; align-items: center; gap: 10px; padding: 6px 8px 6px 12px; border-radius: 6px;
      background: var(--tooltip-bg); border: 1px solid var(--lvl-error); color: var(--text); box-shadow: var(--shadow);
    }

    /* Drop guides (VS compass and edge guides) and the preview of the drop result. */
    .imt-drop-guides { position: absolute; inset: 0; pointer-events: none; z-index: 1000; display: none; }
    .imt-drop-guides.on { display: block; }
    .imt-guide {
      position: absolute; display: flex; align-items: center; justify-content: center; font-size: 11px;
      color: var(--accent); background: var(--bg-1); border: 1px solid var(--accent); border-radius: 4px;
      box-shadow: 0 2px 8px rgba(0, 0, 0, 0.35); opacity: 0.92;
    }
    .imt-guide-center::after { content: ''; width: 14px; height: 14px; border: 2px solid var(--accent); border-radius: 2px; }
    .imt-guide.hot { background: var(--accent); color: var(--accent-ink); }
    .imt-guide-center.hot::after { border-color: var(--accent-ink); }
    .imt-dockview .dv-drop-target-anchor, .imt-dockview .dv-drop-target-selection {
      border: 2px dashed var(--accent) !important; background-color: var(--accent-soft) !important; border-radius: 2px;
    }

    /* VS-style tabs: a thin accent line on the active document tab, muted inactive tabs, a pin on pinned tabs. */
    .imt-dockview .dv-groupview.dv-active-group .dv-tab.dv-active-tab { box-shadow: inset 0 2px 0 var(--accent); background: var(--bg-1); }
    .imt-dockview .dv-groupview.dv-inactive-group .dv-tab.dv-active-tab { box-shadow: inset 0 2px 0 var(--border-3); }
    .imt-tab-pin { display: none; font-size: 10px; filter: grayscale(1); }
    .imt-tab.pinned .imt-tab-pin { display: inline; }
    .imt-tab-menu { width: 14px; text-align: center; font-size: 11px; color: var(--faint); visibility: hidden; border-radius: 3px; }
    .dv-tab:hover .imt-tab-menu, .dv-tab.dv-active-tab .imt-tab-menu, .dv-tab:focus-visible .imt-tab-menu { visibility: visible; }
    /* Only the active tab carries the menu chevron, so inactive tabs stay as narrow as their label and close button. */
    .dv-tab:not(.dv-active-tab) .imt-tab-menu { display: none; }
    .imt-tab-menu:hover { background: var(--bg-hover); color: var(--text); }
    /* Header and tab actions keep a full 24x24 target (WCAG 2.5.8) in a crowded header: never shrink. */
    .imt-dock-action { flex: none; min-width: 24px; min-height: 24px; }
    .imt-dock-action[hidden] { display: none; }

    /* Full screen (Shift+Alt+Enter): only the document well; the app's chrome is hidden. */
    html.imt-full-screen imt-title-bar,
    html.imt-full-screen imt-activity-bar,
    html.imt-full-screen imt-sidebar,
    html.imt-full-screen imt-status-bar,
    html.imt-full-screen imt-filter-bar,
    html.imt-full-screen .imt-ah-bar { display: none !important; }
  `,
})
export class DockHost implements OnDestroy {
  protected readonly dock = inject(DockService);
  private readonly views = inject(ViewRegistry);
  private readonly appRef = inject(ApplicationRef);
  private readonly injector = inject(EnvironmentInjector);
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);
  private readonly keys = inject(DockKeys);
  private readonly mount = viewChild.required<ElementRef<HTMLElement>>('mount');
  private readonly flyoutBody = viewChild<ElementRef<HTMLElement>>('flyoutBody');
  protected readonly sides: readonly Side[] = ['left', 'right', 'bottom'];

  protected barLabel(side: Side): string {
    switch (side) {
      case 'left':
        return $localize`:@@dock.host.barLeft:Auto-hidden tool windows (left)`;
      case 'right':
        return $localize`:@@dock.host.barRight:Auto-hidden tool windows (right)`;
      default:
        return $localize`:@@dock.host.barBottom:Auto-hidden tool windows (bottom)`;
    }
  }

  protected pinLabel(title: string): string {
    return $localize`:@@dock.host.pinLabel:Pin ${title}:title:`;
  }

  protected hideLabel(title: string): string {
    return $localize`:@@dock.host.hideLabel:Hide ${title}:title:`;
  }
  private leaveTimer: ReturnType<typeof setTimeout> | null = null;
  /** Slid-out views keep their component while they stay auto-hidden. */
  private readonly renderers = new Map<string, AngularContentRenderer>();
  private readonly offKeys: () => void;

  protected readonly bars = computed<Record<Side, AutoHideItem[]>>(() => {
    const s = this.dock.autoHide();
    return { left: itemsOn(s, 'left'), right: itemsOn(s, 'right'), bottom: itemsOn(s, 'bottom') };
  });
  protected readonly open = computed(() => {
    const s = this.dock.autoHide();
    return s.items.find((i) => i.id === s.open) ?? null;
  });

  constructor() {
    afterNextRender(() => this.dock.attach(this.mount().nativeElement));
    this.offKeys = this.keys.listen(document);
    // Mount the slid-out view's component into the flyout body.
    effect(() => {
      const item = this.open();
      const body = this.flyoutBody()?.nativeElement;
      const ids = new Set(this.dock.autoHide().items.map((i) => i.id));
      untracked(() => {
        for (const [id, r] of this.renderers) {
          if (!ids.has(id)) {
            r.dispose();
            this.renderers.delete(id);
          }
        }
        if (!item || !body) return;
        let renderer = this.renderers.get(item.id);
        if (!renderer) {
          const view = this.views.find(item.id);
          if (!view) return;
          renderer = new AngularContentRenderer(view.component, this.appRef, this.injector, view.title);
          renderer.init({} as never);
          this.renderers.set(item.id, renderer);
        }
        body.replaceChildren(renderer.element);
      });
    });
    const onPointerDown = (e: PointerEvent) => {
      const t = e.target as Element | null;
      if (this.open() && !t?.closest?.('.imt-ah-flyout, .imt-ah-bar, .menu')) this.dock.autoHideEvent({ type: 'blur' });
    };
    document.addEventListener('pointerdown', onPointerDown, true);
    this.cleanup.push(() => document.removeEventListener('pointerdown', onPointerDown, true));
    // Escape anywhere hides a slide-out (one opened by selecting an event leaves the focus where it was), unless
    // something else (a menu, a dialog, an editor) used the key.
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !e.defaultPrevented && this.open()) this.dock.autoHideEvent({ type: 'escape' });
    };
    document.addEventListener('keydown', onKeyDown);
    this.cleanup.push(() => document.removeEventListener('keydown', onKeyDown));
  }

  private readonly cleanup: (() => void)[] = [];

  protected hover(id: string): void {
    this.cancelLeave();
    this.dock.autoHideEvent({ type: 'hover', id });
  }

  protected click(id: string, e: MouseEvent): void {
    this.cancelLeave();
    // A click from the keyboard (Enter/Space on the focused tab) has no pointer position.
    const keyboard = e.detail === 0;
    this.dock.autoHideEvent(keyboard ? { type: 'activate', id } : { type: 'click', id });
    if (keyboard) afterNextRender(() => this.host.nativeElement.querySelector<HTMLElement>('.imt-ah-flyout')?.focus(), { injector: this.injector });
  }

  protected leaveSoon(): void {
    this.cancelLeave();
    this.leaveTimer = setTimeout(() => this.dock.autoHideEvent({ type: 'leave' }), HOVER_HIDE_MS);
  }

  protected cancelLeave(): void {
    if (this.leaveTimer !== null) clearTimeout(this.leaveTimer);
    this.leaveTimer = null;
  }

  protected escape(): void {
    const id = this.open()?.id;
    this.dock.autoHideEvent({ type: 'escape' });
    if (id) afterNextRender(() => this.host.nativeElement.querySelector<HTMLElement>(`.imt-ah-tab[data-ah-id="${CSS.escape(id)}"]`)?.focus(), { injector: this.injector });
  }

  protected focusOut(e: FocusEvent): void {
    const next = e.relatedTarget as Element | null;
    if (next && !next.closest('.imt-ah-flyout, .imt-ah-bar, .menu')) this.dock.autoHideEvent({ type: 'blur' });
  }

  protected retry(): void {
    const notice = this.dock.notice();
    this.dock.notice.set(null);
    notice?.retry?.();
  }

  protected startResize(e: PointerEvent, item: AutoHideItem): void {
    e.preventDefault();
    const start = { x: e.clientX, y: e.clientY, size: item.size };
    const move = (m: PointerEvent) => {
      const delta = item.side === 'left' ? m.clientX - start.x : item.side === 'right' ? start.x - m.clientX : start.y - m.clientY;
      this.dock.autoHideEvent({ type: 'resize', id: item.id, size: start.size + delta });
    };
    const up = () => {
      document.removeEventListener('pointermove', move);
      document.removeEventListener('pointerup', up);
    };
    document.addEventListener('pointermove', move);
    document.addEventListener('pointerup', up);
  }

  ngOnDestroy(): void {
    this.cancelLeave();
    this.offKeys();
    this.cleanup.forEach((c) => c());
    for (const r of this.renderers.values()) r.dispose();
    this.renderers.clear();
    this.dock.detach();
  }
}
