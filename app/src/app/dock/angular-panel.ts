import { ApplicationRef, ComponentRef, createComponent, EnvironmentInjector, Type } from '@angular/core';
import type { GroupPanelPartInitParameters, IContentRenderer } from 'dockview';

/**
 * Adapter that renders an Angular standalone component inside a dockview panel.
 * The component is created lazily with `createComponent`, attached to the `ApplicationRef` so it is
 * change-detected, and destroyed (and detached) when dockview disposes the panel.
 *
 * TODO(popout): dockview can pop panels out to OS windows; components would then need to be
 * re-parented to the new document. Out of scope for now.
 */
export class AngularContentRenderer implements IContentRenderer {
  readonly element = document.createElement('div');
  private ref: ComponentRef<unknown> | null = null;
  private disposed = false;
  private mounted = false;
  private visibility?: { dispose(): void };

  constructor(
    private readonly load: () => Promise<Type<unknown>>,
    private readonly appRef: ApplicationRef,
    private readonly injector: EnvironmentInjector,
    private readonly label = 'view',
  ) {
    this.element.className = 'imt-panel';
  }

  /** The component is created when the panel is first visible, so hidden tabs cost nothing until opened. */
  init(params: GroupPanelPartInitParameters): void {
    const api = params.api;
    if (!api || api.isVisible) {
      void this.mount();
      return;
    }
    this.visibility = api.onDidVisibilityChange((e) => {
      if (e.isVisible) void this.mount();
    });
  }

  private async mount(): Promise<void> {
    if (this.mounted || this.disposed) return;
    this.mounted = true;
    this.visibility?.dispose();
    try {
      const type = await this.load();
      if (this.disposed) return;
      const ref = createComponent(type, { environmentInjector: this.injector });
      this.appRef.attachView(ref.hostView);
      this.element.appendChild(ref.location.nativeElement as Node);
      this.ref = ref;
    } catch {
      if (this.disposed) return;
      const msg = document.createElement('div');
      msg.className = 'imt-panel-error';
      msg.setAttribute('role', 'alert');
      msg.textContent = `The ${this.label} panel could not be loaded.`;
      this.element.appendChild(msg);
    }
  }

  dispose(): void {
    this.disposed = true;
    this.visibility?.dispose();
    if (this.ref) {
      this.appRef.detachView(this.ref.hostView);
      this.ref.destroy();
      this.ref = null;
    }
    this.element.remove();
  }
}

/** Placeholder for a panel in a saved layout whose view no longer exists (e.g. a removed plugin). */
export class MissingViewRenderer implements IContentRenderer {
  readonly element = document.createElement('div');
  constructor(id: string) {
    this.element.className = 'imt-panel imt-panel-error';
    this.element.textContent = `The view "${id}" is not available. Close this tab or reset the layout.`;
  }
  init(): void {}
}
