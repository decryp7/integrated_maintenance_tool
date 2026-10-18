import type { DockviewApi, DockviewGroupPanel, IGroupHeaderProps, IHeaderActionsRenderer } from 'dockview';

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
  float: 'M8 8h12v12H8zM4 16V4h12',
  dock: 'M4 5h16v14H4zM4 10h16',
  popout: 'M14 4h6v6M20 4l-8 8M18 14v6H4V6h6',
  autoHide: 'M12 17v5M8 3h8l-1 7 3 3H6l3-3z',
  menu: 'M6 9l6 6 6-6',
};

export interface GroupActionsHost {
  /** Floats a docked group, or docks a floating one back into the grid. */
  toggleFloat(group: DockviewGroupPanel): void;
  /** Pops the group's active panel out into a window of its own. */
  popOut(group: DockviewGroupPanel): void;
  /** Auto-hides the group's active tool window. */
  autoHide(group: DockviewGroupPanel): void;
  /** Is the group's active panel a tool window (which can auto-hide)? */
  isTool(group: DockviewGroupPanel): boolean;
  canPopOut(): boolean;
  /** Opens the active panel's menu under `anchor`. */
  openMenu(group: DockviewGroupPanel, anchor: HTMLElement): void;
}

/** Panel menu, auto hide, pop out, float/dock and maximize buttons at the right of every group's tab strip. */
export class GroupActions implements IHeaderActionsRenderer {
  readonly element = document.createElement('div');
  private readonly menu = this.button();
  private readonly autoHide = this.button();
  private readonly popout = this.button();
  private readonly maximize = this.button();
  private readonly float = this.button();
  private cleanup: (() => void)[] = [];
  private group?: DockviewGroupPanel;
  private api?: DockviewApi;

  constructor(private readonly host: GroupActionsHost) {
    this.element.className = 'imt-dock-actions';
    this.menu.setAttribute('aria-haspopup', 'menu');
    this.set(this.menu, $localize`:@@dock.groupActions.panelMenu:Panel menu`, ICONS.menu);
    this.set(this.autoHide, $localize`:@@dock.groupActions.autoHide:Auto hide`, ICONS.autoHide);
    this.set(this.popout, $localize`:@@dock.groupActions.popOut:Pop out to window`, ICONS.popout);
    this.element.append(this.menu, this.autoHide, this.popout, this.float, this.maximize);
  }

  private button(): HTMLButtonElement {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'imt-dock-action';
    return b;
  }

  init(params: IGroupHeaderProps): void {
    this.group = params.group as DockviewGroupPanel;
    this.api = params.containerApi;
    this.maximize.addEventListener('click', () => this.toggleMaximize());
    this.float.addEventListener('click', () => this.host.toggleFloat(this.group!));
    this.popout.addEventListener('click', () => this.host.popOut(this.group!));
    this.autoHide.addEventListener('click', () => this.host.autoHide(this.group!));
    this.menu.addEventListener('click', () => this.host.openMenu(this.group!, this.menu));
    const refresh = () => this.refresh();
    const subs = [params.api.onDidLocationChange(refresh), params.containerApi.onDidMaximizedGroupChange(refresh), params.api.onDidActivePanelChange(refresh)];
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
    const floating = group.api.location.type === 'floating';
    this.set(this.float, floating ? $localize`:@@dock.groupActions.dockBack:Dock panel back into the window` : $localize`:@@dock.groupActions.float:Float panel`, floating ? ICONS.dock : ICONS.float);
    this.autoHide.hidden = floating || !this.host.isTool(group);
    this.popout.hidden = !this.host.canPopOut();
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
