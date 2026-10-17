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
};

export interface GroupActionsHost {
  /** Floats a docked group, or docks a floating one back into the grid. */
  toggleFloat(group: DockviewGroupPanel): void;
}

/** Maximize and float/dock buttons shown at the right of every group's tab strip. */
export class GroupActions implements IHeaderActionsRenderer {
  readonly element = document.createElement('div');
  private readonly maximize = this.button();
  private readonly float = this.button();
  private cleanup: (() => void)[] = [];
  private group?: DockviewGroupPanel;
  private api?: DockviewApi;

  constructor(private readonly host: GroupActionsHost) {
    this.element.className = 'imt-dock-actions';
    this.element.append(this.float, this.maximize);
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
    const refresh = () => this.refresh();
    const subs = [params.api.onDidLocationChange(refresh), params.containerApi.onDidMaximizedGroupChange(refresh)];
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
    this.set(this.maximize, maximized ? 'Restore panel size' : 'Maximize panel', maximized ? ICONS.restore : ICONS.maximize);
    const floating = group.api.location.type === 'floating';
    this.set(this.float, floating ? 'Dock panel back into the window' : 'Float panel', floating ? ICONS.dock : ICONS.float);
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
