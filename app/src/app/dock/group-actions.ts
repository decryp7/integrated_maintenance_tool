import type { DockviewGroupPanel, IGroupHeaderProps, IHeaderActionsRenderer } from 'dockview';
import { ICON_PATHS } from '../shared/material/debrief-icons';

const SVG_NS = 'http://www.w3.org/2000/svg';

function icon(d: string): SVGElement {
  const svg = document.createElementNS(SVG_NS, 'svg');
  svg.setAttribute('width', '13');
  svg.setAttribute('height', '13');
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

  constructor(private readonly host: GroupActionsHost) {
    this.element.className = 'debrief-dock-actions';
    this.menu.setAttribute('aria-haspopup', 'menu');
    this.set(this.menu, $localize`:@@dock.groupActions.panelMenu:Panel menu`, ICONS.more);
    this.autoHide.hidden = true;
    this.autoHide.className += ' debrief-dock-autohide';
    this.autoHide.setAttribute('aria-haspopup', 'false');
    this.element.append(...(host.autoHide ? [this.autoHide] : []), this.menu, this.maximize);
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
    if (!show || !panel) return;
    const title = panel.title ?? panel.id;
    this.autoHide.setAttribute('aria-pressed', 'false');
    this.autoHide.setAttribute('aria-label', $localize`:@@dock.groupActions.autoHide:Auto-hide ${title}:title:`);
    this.autoHide.title = $localize`:@@dock.groupActions.autoHideTitle:Auto-hide ${title}:title: (Ctrl+Alt+H). Right-click to choose the left, right or bottom edge.`;
    this.autoHide.replaceChildren(icon(ICONS.pin));
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
