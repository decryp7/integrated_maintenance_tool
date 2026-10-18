import type { DockviewPanelApi, ITabRenderer, TabPartInitParameters } from 'dockview';

/** Component name dockview uses for the tab renderer of every panel. */
export const TAB_COMPONENT = 'imt-tab';

/** Opens the panel menu of a tab (the ▾); `anchor` is where the menu appears. */
export type OpenPanelMenu = (id: string, anchor: HTMLElement) => void;

/**
 * Tab content: a pin mark (pinned tabs), the title, the panel menu "▾" and a close "×". The default dockview tab
 * nests buttons inside the `role=tab` element, which assistive tech reports as nested interactive controls. Here
 * the ▾ and × are decorative for the mouse; keyboard users open the menu of a focused tab with Shift+F10 (or the
 * Menu key) and close it with Delete.
 */
export class PanelTab implements ITabRenderer {
  readonly element = document.createElement('div');
  private readonly pin = document.createElement('span');
  private readonly label = document.createElement('span');
  private readonly menu = document.createElement('span');
  private readonly close = document.createElement('span');
  private api?: DockviewPanelApi;
  private subscription?: { dispose(): void };

  constructor(private readonly openMenu: OpenPanelMenu | null = null) {
    this.element.className = 'imt-tab';
    this.pin.className = 'imt-tab-pin';
    this.pin.setAttribute('aria-hidden', 'true');
    this.pin.textContent = '📌';
    this.label.className = 'imt-tab-label';
    this.menu.className = 'imt-tab-menu';
    this.menu.setAttribute('aria-hidden', 'true');
    this.menu.textContent = '▾';
    this.menu.title = $localize`:@@dock.panelTab.menu:Panel menu (Shift+F10)`;
    this.close.className = 'imt-tab-close';
    this.close.setAttribute('aria-hidden', 'true');
    this.close.textContent = '×';
    this.close.title = $localize`:@@dock.panelTab.close:Close (Delete)`;
    this.element.append(this.pin, this.label, ...(openMenu ? [this.menu] : []), this.close);
  }

  init(params: TabPartInitParameters): void {
    this.api = params.api;
    this.element.dataset['panelId'] = params.api.id;
    this.label.textContent = params.title;
    this.subscription = params.api.onDidTitleChange((e) => (this.label.textContent = e.title));
    this.close.addEventListener('click', (e) => {
      e.stopPropagation();
      this.api?.close();
    });
    this.menu.addEventListener('click', (e) => {
      e.stopPropagation();
      if (this.api) this.openMenu?.(this.api.id, this.element.closest<HTMLElement>('[role="tab"]') ?? this.element);
    });
    // Do not start a drag when pressing the close or menu affordance.
    this.close.addEventListener('pointerdown', (e) => e.stopPropagation());
    this.menu.addEventListener('pointerdown', (e) => e.stopPropagation());
  }

  dispose(): void {
    this.subscription?.dispose();
  }
}

/** Id of the panel whose tab contains `target`, if the event came from a tab. */
export function tabPanelId(target: EventTarget | null): string | null {
  const tab = (target as HTMLElement | null)?.closest?.('[role="tab"]');
  return tab?.querySelector<HTMLElement>('.imt-tab')?.dataset['panelId'] ?? null;
}
