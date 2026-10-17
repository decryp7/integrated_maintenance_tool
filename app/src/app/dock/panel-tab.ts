import type { DockviewPanelApi, ITabRenderer, TabPartInitParameters } from 'dockview';

/** Component name dockview uses for the tab renderer of every panel. */
export const TAB_COMPONENT = 'imt-tab';

/**
 * Tab content: the title and a close "×". The default dockview tab nests a button inside the
 * `role=tab` element, which assistive tech reports as nested interactive controls. Here the × is
 * decorative for the mouse; keyboard users close a focused tab with Delete (see `closeFocusedTab`).
 */
export class PanelTab implements ITabRenderer {
  readonly element = document.createElement('div');
  private readonly label = document.createElement('span');
  private readonly close = document.createElement('span');
  private api?: DockviewPanelApi;
  private subscription?: { dispose(): void };

  constructor() {
    this.element.className = 'imt-tab';
    this.label.className = 'imt-tab-label';
    this.close.className = 'imt-tab-close';
    this.close.setAttribute('aria-hidden', 'true');
    this.close.textContent = '×';
    this.close.title = 'Close (Delete)';
    this.element.append(this.label, this.close);
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
    // Do not start a drag when pressing the close affordance.
    this.close.addEventListener('pointerdown', (e) => e.stopPropagation());
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
