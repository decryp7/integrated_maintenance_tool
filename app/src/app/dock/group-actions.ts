import type { DockviewGroupPanel, IGroupHeaderProps, IHeaderActionsRenderer } from 'dockview';

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
};

export interface GroupActionsHost {
  /** Opens the active panel's menu (Float, Pop out, Dock, Auto hide, Close...) under `anchor`. */
  openMenu(group: DockviewGroupPanel, anchor: HTMLElement): void;
}

/**
 * The two buttons at the right of every group's tab strip: the panel menu (one place for Float, Pop out, Dock, Auto
 * hide and Close) and Maximize / Restore.
 */
export class GroupActions implements IHeaderActionsRenderer {
  readonly element = document.createElement('div');
  private readonly menu = this.button();
  private readonly maximize = this.button();
  private cleanup: (() => void)[] = [];
  private group?: DockviewGroupPanel;

  constructor(private readonly host: GroupActionsHost) {
    this.element.className = 'imt-dock-actions';
    this.menu.setAttribute('aria-haspopup', 'menu');
    this.set(this.menu, $localize`:@@dock.groupActions.panelMenu:Panel menu`, ICONS.more);
    this.element.append(this.menu, this.maximize);
  }

  private button(): HTMLButtonElement {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'imt-dock-action';
    return b;
  }

  init(params: IGroupHeaderProps): void {
    this.group = params.group as DockviewGroupPanel;
    this.maximize.addEventListener('click', () => this.toggleMaximize());
    this.menu.addEventListener('click', () => this.host.openMenu(this.group!, this.menu));
    const refresh = () => this.refresh();
    const subs = [params.containerApi.onDidMaximizedGroupChange(refresh)];
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
    this.set(this.maximize, maximized ? $localize`:@@dock.groupActions.restore:Restore panel size` : $localize`:@@dock.groupActions.maximize:Maximize panel`, maximized ? ICONS.restore : ICONS.maximize);
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
