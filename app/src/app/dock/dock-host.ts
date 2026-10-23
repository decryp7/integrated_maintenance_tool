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
import { Icon } from '../shared/icon';

/** What a slid-out panel keeps open when it is pressed or focused: itself, its bar, menus, and the overlays and dialogs it opened (lists, pickers, reviews). */
const KEEP_OPEN = '.debrief-ah-flyout, .debrief-ah-bar, .menu, .cdk-overlay-container, debrief-dialog-host';

/** How long the pointer may be away from a slid-out (hover-opened) tool window before it hides again. */
export const HOVER_HIDE_MS = 300;

/**
 * Hosts the dockview instance managed by `DockService`, with Visual Studio's extras around it: the auto-hide side
 * bars and their slide-out, the Ctrl+Tab navigator, inline dock notices and the full-screen mode.
 */
@Component({
  selector: 'debrief-dock-host',
  changeDetection: ChangeDetectionStrategy.OnPush,
  encapsulation: ViewEncapsulation.None,
  imports: [DockSwitcher, Icon],
  template: `
    @for (side of sides; track side) {
      @if (bars()[side].length) {
        <nav class="debrief-ah-bar debrief-ah-{{ side }}" [attr.aria-label]="barLabel(side)">
          @for (item of bars()[side]; track item.id) {
            <button
              type="button"
              class="debrief-ah-tab"
              [class.open]="open()?.id === item.id"
              [attr.aria-expanded]="open()?.id === item.id"
              [attr.aria-controls]="'debrief-ah-flyout'"
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
    <div class="debrief-dock-center">
      <div #mount class="debrief-dock-mount"></div>
      @if (open(); as item) {
        <section
          id="debrief-ah-flyout"
          class="debrief-ah-flyout debrief-ah-flyout-{{ item.side }}"
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
          <header class="debrief-ah-head">
            <span class="debrief-ah-title">{{ item.title }}</span>
            <button type="button" class="debrief-dock-action debrief-ah-pin" [attr.aria-label]="pinLabel(item.title)" aria-pressed="true" [attr.title]="pinTitle()" (click)="dock.pin(item.id)">
              <debrief-icon name="unpin" [size]="16" [stroke]="2" />
            </button>
            <button type="button" class="debrief-dock-action" [attr.aria-label]="hideLabel(item.title)" title="Hide (Esc)" i18n-title="@@dock.host.hideTitle" (click)="escape()">
              <svg width="13" height="13" viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>
            </button>
          </header>
          <div #flyoutBody class="debrief-ah-body"></div>
          <div class="debrief-ah-grip" aria-hidden="true" (pointerdown)="startResize($event, item)"></div>
        </section>
      }
      @if (dock.notice(); as notice) {
        <div class="debrief-dock-notice" role="alert">
          <span>{{ notice.message }}</span>
          @if (notice.retry) {
            <button type="button" class="btn" (click)="retry()" i18n="@@dock.host.retry">Retry</button>
          }
          <button type="button" class="btn" (click)="dock.notice.set(null)" i18n="@@dock.host.dismiss">Dismiss</button>
        </div>
      }
    </div>
    @if (dock.switcher()) {
      <debrief-dock-switcher />
    }
  `,
  styles: `
    debrief-dock-host {
      display: grid; flex: 1; min-height: 0; min-width: 0; position: relative;
      grid-template-columns: auto minmax(0, 1fr) auto; grid-template-rows: minmax(0, 1fr) auto;
      grid-template-areas: 'left center right' 'bottom bottom bottom';
    }
    .debrief-dock-center { grid-area: center; position: relative; min-width: 0; min-height: 0; display: flex; }
    .debrief-dock-mount { flex: 1; min-width: 0; min-height: 0; position: relative; }

    /* Auto-hide side bars (VS): vertical labels on the sides, horizontal at the bottom. */
    .debrief-ah-bar { display: flex; gap: 2px; padding: 2px; background: var(--bg-0); }
    .debrief-ah-left { grid-area: left; flex-direction: column; border-right: 1px solid var(--border); }
    .debrief-ah-right { grid-area: right; flex-direction: column; border-left: 1px solid var(--border); }
    .debrief-ah-bottom { grid-area: bottom; flex-direction: row; border-top: 1px solid var(--border); }
    .debrief-ah-tab {
      border: 0; background: transparent; color: var(--muted); font: inherit; font-size: var(--fs-sm); cursor: pointer;
      padding: 8px 4px; border-radius: 3px; white-space: nowrap;
    }
    .debrief-ah-left .debrief-ah-tab, .debrief-ah-right .debrief-ah-tab { writing-mode: vertical-rl; }
    .debrief-ah-left .debrief-ah-tab { transform: rotate(180deg); }
    .debrief-ah-bottom .debrief-ah-tab { padding: 3px 10px; }
    .debrief-ah-tab:hover, .debrief-ah-tab.open { color: var(--text); background: var(--bg-hover); box-shadow: inset 0 -2px 0 var(--accent); }
    .debrief-ah-left .debrief-ah-tab:hover, .debrief-ah-left .debrief-ah-tab.open { box-shadow: inset 2px 0 0 var(--accent); }
    .debrief-ah-right .debrief-ah-tab:hover, .debrief-ah-right .debrief-ah-tab.open { box-shadow: inset -2px 0 0 var(--accent); }
    .debrief-ah-tab:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }

    .debrief-ah-flyout {
      position: absolute; z-index: 990; display: flex; flex-direction: column; background: var(--bg-1);
      border: 1px solid var(--border-3); box-shadow: var(--shadow); outline: none;
    }
    .debrief-ah-flyout-left { left: 0; top: 0; bottom: 0; animation: debrief-slide-left 140ms ease-out; }
    .debrief-ah-flyout-right { right: 0; top: 0; bottom: 0; animation: debrief-slide-right 140ms ease-out; }
    .debrief-ah-flyout-bottom { left: 0; right: 0; bottom: 0; animation: debrief-slide-up 140ms ease-out; }
    @keyframes debrief-slide-left { from { transform: translateX(-24px); opacity: 0.4; } }
    @keyframes debrief-slide-right { from { transform: translateX(24px); opacity: 0.4; } }
    @keyframes debrief-slide-up { from { transform: translateY(24px); opacity: 0.4; } }
    @media (prefers-reduced-motion: reduce) { .debrief-ah-flyout { animation: none; } }
    .debrief-ah-head {
      display: flex; align-items: center; gap: 2px; height: 30px; padding: 0 4px 0 10px; flex: none;
      background: var(--bg-0); border-bottom: 1px solid var(--border); box-shadow: inset 0 2px 0 var(--accent);
    }
    .debrief-ah-title { flex: 1; font-size: var(--fs-sm); color: var(--text); }
    .debrief-ah-body { flex: 1; min-height: 0; display: flex; }
    .debrief-ah-body > .debrief-panel { flex: 1; }
    .debrief-ah-grip { position: absolute; background: transparent; }
    .debrief-ah-flyout-left .debrief-ah-grip { right: -3px; top: 0; bottom: 0; width: 6px; cursor: ew-resize; }
    .debrief-ah-flyout-right .debrief-ah-grip { left: -3px; top: 0; bottom: 0; width: 6px; cursor: ew-resize; }
    .debrief-ah-flyout-bottom .debrief-ah-grip { top: -3px; left: 0; right: 0; height: 6px; cursor: ns-resize; }

    .debrief-dock-notice {
      position: absolute; z-index: 995; inset-inline-start: 50%; top: 8px; transform: translateX(-50%); max-width: 80%;
      display: flex; align-items: center; gap: 10px; padding: 6px 8px 6px 12px; border-radius: 6px;
      background: var(--tooltip-bg); border: 1px solid var(--lvl-error); color: var(--text); box-shadow: var(--shadow);
    }

    /* Drop guides (VS compass and edge guides) and the preview of the drop result. */
    .debrief-drop-guides { position: absolute; inset: 0; pointer-events: none; z-index: 1000; display: none; }
    .debrief-drop-guides.on { display: block; }
    .debrief-guide {
      position: absolute; display: flex; align-items: center; justify-content: center; font-size: var(--fs-xs);
      color: var(--accent-text); background: var(--bg-1); border: 1px solid var(--accent); border-radius: 4px;
      box-shadow: var(--shadow-pop); opacity: 0.92;
    }
    .debrief-guide-center::after { content: ''; width: 14px; height: 14px; border: 2px solid var(--accent); border-radius: 2px; }
    .debrief-guide.hot { background: var(--accent); color: var(--accent-ink); }
    .debrief-guide-center.hot::after { border-color: var(--accent-ink); }
    /* The cue of a tab dragged off the drop zones: a ghost of the floating panel (or new window) and the hint. */
    .debrief-float-cue {
      position: fixed; z-index: 1001; pointer-events: none; display: flex; align-items: flex-start; justify-content: center;
      padding-top: 10px; border: 2px solid var(--accent); border-radius: 6px; background: var(--accent-soft);
      box-shadow: var(--shadow); opacity: 0.6; animation: debrief-float-cue-in 120ms ease-out;
    }
    .debrief-float-cue[hidden] { display: none; }
    /* Off every guide inside the window: only a small hint by the pointer, and the dragged tab chip dims. */
    .debrief-float-cue[data-outcome='cancel'] {
      border: 0; background: none; box-shadow: none; padding: 0; opacity: 1; overflow: visible; justify-content: flex-start;
    }
    .debrief-float-cue[data-outcome='cancel'] .debrief-float-cue-hint { background: var(--bg-2); color: var(--text); border: 1px solid var(--border-3); }
    body.debrief-drag-cancel .dv-tab-ghost-drag { opacity: 0.5 !important; }
    /* dockview holds the dragged tab chip just below the pointer, over the guide being aimed at: move it right of and
       below the pointer so the guide stays visible. */
    .dv-tab-ghost-drag { margin: 2px 0 0 42px !important; }
    .debrief-float-cue-hint {
      padding: 4px 10px; border-radius: 4px; font-size: var(--fs-sm); font-weight: 500; white-space: nowrap;
      background: var(--accent); color: var(--accent-ink); box-shadow: var(--shadow-pop);
    }
    @keyframes debrief-float-cue-in { from { opacity: 0; } }
    @media (prefers-reduced-motion: reduce) { .debrief-float-cue { animation: none; } }
    .debrief-dockview .dv-drop-target-anchor, .debrief-dockview .dv-drop-target-selection {
      border: 2px dashed var(--accent) !important; background-color: var(--accent-soft) !important; border-radius: 2px;
    }

    /* VS-style tabs: a thin accent line on the active document tab, muted inactive tabs, a pin on pinned tabs. */
    .debrief-dockview .dv-groupview.dv-active-group .dv-tab.dv-active-tab { box-shadow: inset 0 2px 0 var(--accent); background: var(--bg-1); }
    .debrief-dockview .dv-groupview.dv-inactive-group .dv-tab.dv-active-tab { box-shadow: inset 0 2px 0 var(--border-3); }
    .debrief-tab-pin { display: none; font-size: var(--fs-2xs); filter: grayscale(1); }
    .debrief-tab.pinned .debrief-tab-pin { display: inline; }
    .debrief-tab-menu { width: 14px; text-align: center; font-size: var(--fs-xs); color: var(--faint); visibility: hidden; border-radius: 3px; }
    .dv-tab:hover .debrief-tab-menu, .dv-tab.dv-active-tab .debrief-tab-menu, .dv-tab:focus-visible .debrief-tab-menu { visibility: visible; }
    /* Only the active tab carries the menu chevron, so inactive tabs stay as narrow as their label and close button. */
    .dv-tab:not(.dv-active-tab) .debrief-tab-menu { display: none; }
    .debrief-tab-menu:hover { background: var(--bg-hover); color: var(--text); }
    /* Header and tab actions keep a full 24x24 target (WCAG 2.5.8) in a crowded header: never shrink. */
    .debrief-dock-action { flex: none; min-width: 24px; min-height: 24px; }
    .debrief-dock-action[hidden] { display: none; }

    /* Full screen (Shift+Alt+Enter): only the document well; the app's chrome is hidden. */
    html.debrief-full-screen debrief-title-bar,
    html.debrief-full-screen debrief-activity-bar,
    html.debrief-full-screen debrief-sidebar,
    html.debrief-full-screen debrief-status-bar,
    html.debrief-full-screen debrief-filter-bar,
    html.debrief-full-screen .debrief-ah-bar { display: none !important; }
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

  /** The Pin tooltip with the user's current binding of the auto-hide command. */
  protected pinTitle(): string {
    const keys = this.dock.autoHideShortcut();
    return keys ? $localize`:@@dock.host.pinTitleKeys:Pin (dock it again, ${keys}:keys:)` : $localize`:@@dock.host.pinTitle:Pin (dock it again)`;
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
    // effect-state: mounts and disposes view renderers in the DOM flyout (side effect, not derived state)
    effect(() => {
      const item = this.open();
      const body = this.flyoutBody()?.nativeElement;
      const ids = new Set(this.dock.autoHide().items.map((i) => i.id));
      untracked(() => {
        for (const [id, r] of this.renderers) {
          if (!ids.has(id)) {
            r.dispose();
            this.renderers.delete(id);
          } else r.setVisible(id === item?.id && !!body);
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
      if (this.open() && !t?.closest?.(KEEP_OPEN)) this.dock.autoHideEvent({ type: 'blur' });
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
    if (keyboard) afterNextRender(() => this.host.nativeElement.querySelector<HTMLElement>('.debrief-ah-flyout')?.focus(), { injector: this.injector });
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
    if (id) afterNextRender(() => this.host.nativeElement.querySelector<HTMLElement>(`.debrief-ah-tab[data-ah-id="${CSS.escape(id)}"]`)?.focus(), { injector: this.injector });
  }

  protected focusOut(e: FocusEvent): void {
    const next = e.relatedTarget as Element | null;
    if (next && !next.closest(KEEP_OPEN)) this.dock.autoHideEvent({ type: 'blur' });
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
