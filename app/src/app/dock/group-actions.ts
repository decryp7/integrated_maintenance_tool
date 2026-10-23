import type { DockviewGroupPanel, IGroupHeaderProps, IHeaderActionsRenderer } from 'dockview';
import { ICON_PATHS } from '../shared/material/debrief-icons';

const SVG_NS = 'http://www.w3.org/2000/svg';

function icon(d: string, size = 13): SVGElement {
  const svg = document.createElementNS(SVG_NS, 'svg');
  svg.setAttribute('width', String(size));
  svg.setAttribute('height', String(size));
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('aria-hidden', 'true');
  const path = document.createElementNS(SVG_NS, 'path');
  path.setAttribute('d', d);
  path.setAttribute('fill', 'none');
  path.setAttribute('stroke', 'currentColor');
  path.setAttribute('stroke-width', '2');
  path.setAttribute('stroke-linecap', 'round');
  path.setAttribute('stroke-linejoin', 'round');
  svg.appendChild(path);
  return svg;
}

/** Size of the pin glyph so it carries the visual weight of the header's other icons (13 px). */
export const PIN_SIZE = 17;

const ICONS = {
  maximize: 'M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5',
  restore: 'M9 4v5H4M15 4v5h5M9 20v-5H4M15 20v-5h5',
  more: 'M5.5 12h.01M12 12h.01M18.5 12h.01',
  pin: ICON_PATHS['pin'] ?? '',
};

/** The auto-hide button of a group: shown for a docked tool window, never for documents or floating windows. */
export interface AutoHideButtonHost {
  /** Can the active panel of this group be auto-hidden? Otherwise the button is hidden. */
  available(group: DockviewGroupPanel): boolean;
  /** Auto-hides the active panel to its usual side. */
  run(group: DockviewGroupPanel): void;
  /** Opens the left / right / bottom choice under `anchor` (right-click, the Menu key or Shift+F10 on the button). */
  chooseSide(group: DockviewGroupPanel, anchor: HTMLElement, at?: { x: number; y: number }): void;
  /** The user's current key binding of the auto-hide command, formatted for the platform ("⌃⌥H"); empty when it has none. */
  shortcut?(): string;
}

/**
 * The index a toolbar's roving tabindex moves to for an arrow/Home/End key, or null for any other key. Left and Right
 * wrap around; `count` is the number of visible buttons.
 */
export function rovingIndex(key: string, index: number, count: number): number | null {
  if (count <= 0) return null;
  switch (key) {
    case 'ArrowRight':
      return (index + 1) % count;
    case 'ArrowLeft':
      return (index - 1 + count) % count;
    case 'Home':
      return 0;
    case 'End':
      return count - 1;
    default:
      return null;
  }
}

export interface GroupActionsHost {
  /** Opens the active panel's menu (Float, Pop out, Dock, Auto hide, Close...) under `anchor`. */
  openMenu(group: DockviewGroupPanel, anchor: HTMLElement): void;
  autoHide?: AutoHideButtonHost;
}

/**
 * The buttons at the right of every group's tab strip: Auto-hide (tool windows only), the panel menu (one place for
 * Float, Pop out, Dock, Auto hide and Close) and Maximize / Restore.
 */
export class GroupActions implements IHeaderActionsRenderer {
  readonly element = document.createElement('div');
  private readonly autoHide = this.button();
  private readonly menu = this.button();
  private readonly maximize = this.button();
  private cleanup: (() => void)[] = [];
  private group?: DockviewGroupPanel;
  /** The button that holds the toolbar's one Tab stop (the last one focused, else the first visible). */
  private roving?: HTMLButtonElement;

  constructor(private readonly host: GroupActionsHost) {
    this.element.className = 'debrief-dock-actions';
    // One Tab stop for the whole group header: arrows move between the buttons (roving tabindex).
    this.element.setAttribute('role', 'toolbar');
    this.element.setAttribute('aria-label', $localize`:@@dock.groupActions.toolbar:Panel actions`);
    this.element.addEventListener('focusin', (e) => {
      if (e.target instanceof HTMLButtonElement) {
        this.roving = e.target;
        this.syncRoving();
      }
    });
    this.element.addEventListener('keydown', (e) => this.moveRoving(e));
    this.menu.setAttribute('aria-haspopup', 'menu');
    this.set(this.menu, $localize`:@@dock.groupActions.panelMenu:Panel menu`, ICONS.more);
    this.autoHide.hidden = true;
    this.autoHide.className += ' debrief-dock-autohide';
    this.autoHide.setAttribute('aria-haspopup', 'false');
    this.element.append(...(host.autoHide ? [this.autoHide] : []), this.menu, this.maximize);
    this.syncRoving();
  }

  private visibleButtons(): HTMLButtonElement[] {
    return [...this.element.querySelectorAll<HTMLButtonElement>('button')].filter((b) => !b.hidden);
  }

  /** Gives the Tab stop to the remembered button (the first visible one when it is gone or hidden). */
  private syncRoving(): void {
    const buttons = this.visibleButtons();
    const stop = this.roving && buttons.includes(this.roving) ? this.roving : buttons[0];
    for (const b of this.element.querySelectorAll<HTMLButtonElement>('button')) b.tabIndex = b === stop ? 0 : -1;
  }

  private moveRoving(e: KeyboardEvent): void {
    const buttons = this.visibleButtons();
    const at = buttons.findIndex((b) => b === e.target);
    const to = at < 0 ? null : rovingIndex(e.key, at, buttons.length);
    if (to === null) return;
    e.preventDefault();
    e.stopPropagation();
    buttons[to]?.focus();
  }

  private button(): HTMLButtonElement {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'debrief-dock-action';
    return b;
  }

  init(params: IGroupHeaderProps): void {
    this.group = params.group as DockviewGroupPanel;
    this.maximize.addEventListener('click', () => this.toggleMaximize());
    this.menu.addEventListener('click', () => this.host.openMenu(this.group!, this.menu));
    const ah = this.host.autoHide;
    if (ah) {
      this.autoHide.addEventListener('click', () => ah.run(this.group!));
      // The tooltip names the binding as it is now (the user may have changed it since the header was drawn).
      const fresh = () => this.refreshAutoHideTitle();
      this.autoHide.addEventListener('mouseenter', fresh);
      this.autoHide.addEventListener('focus', fresh);
      // The side choice: right-click, or the Menu key / Shift+F10 on the (focusable) button.
      this.autoHide.addEventListener('contextmenu', (e) => {
        e.preventDefault();
        ah.chooseSide(this.group!, this.autoHide, { x: e.clientX, y: e.clientY });
      });
      this.autoHide.addEventListener('keydown', (e) => {
        if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
          e.preventDefault();
          ah.chooseSide(this.group!, this.autoHide);
        }
      });
    }
    const refresh = () => this.refresh();
    const subs = [
      params.containerApi.onDidMaximizedGroupChange(refresh),
      this.group.api.onDidActivePanelChange(refresh),
      this.group.api.onDidLocationChange(refresh),
    ];
    this.cleanup = subs.map((s) => () => s.dispose());
    this.refresh();
  }

  private toggleMaximize(): void {
    const api = this.group!.api;
    if (api.isMaximized()) api.exitMaximized();
    else api.maximize();
  }

  private refresh(): void {
    const group = this.group!;
    const maximized = group.api.isMaximized();
    this.refreshAutoHide();
    this.set(this.maximize, maximized ? $localize`:@@dock.groupActions.restore:Restore panel size` : $localize`:@@dock.groupActions.maximize:Maximize panel`, maximized ? ICONS.restore : ICONS.maximize);
  }

  /** Shown only for a docked tool window; named after the active panel, with the shortcut in its tooltip. */
  private refreshAutoHide(): void {
    const ah = this.host.autoHide;
    const panel = this.group!.activePanel;
    const show = !!ah && !!panel && ah.available(this.group!);
    this.autoHide.hidden = !show;
    this.syncRoving();
    if (!show || !panel) return;
    const title = panel.title ?? panel.id;
    this.autoHide.setAttribute('aria-pressed', 'false');
    this.autoHide.setAttribute('aria-label', $localize`:@@dock.groupActions.autoHide:Auto-hide ${title}:title:`);
    this.refreshAutoHideTitle();
    // The pin glyph is narrower than the maximize and menu glyphs inside the same box: drawn larger it reads as heavy as they do.
    this.autoHide.replaceChildren(icon(ICONS.pin, PIN_SIZE));
  }

  /** The tooltip, with the binding as it is now (it may change without the header being redrawn). */
  private refreshAutoHideTitle(): void {
    const panel = this.group?.activePanel;
    if (!panel) return;
    const title = panel.title ?? panel.id;
    const keys = this.host.autoHide?.shortcut?.() ?? '';
    this.autoHide.title = keys
      ? $localize`:@@dock.groupActions.autoHideTitleKeys:Auto-hide ${title}:title: (${keys}:keys:). Right-click to choose the left, right or bottom edge.`
      : $localize`:@@dock.groupActions.autoHideTitle:Auto-hide ${title}:title:. Right-click to choose the left, right or bottom edge.`;
  }

  private set(button: HTMLButtonElement, label: string, path: string): void {
    button.setAttribute('aria-label', label);
    button.title = label;
    button.replaceChildren(icon(path));
  }

  dispose(): void {
    this.cleanup.forEach((c) => c());
    this.cleanup = [];
  }
}
