import { ApplicationRef, ChangeDetectorRef, Component, ComponentRef, createComponent, viewChild, ViewContainerRef, EnvironmentInjector, ErrorHandler, InjectionToken, Injector, signal, Type, WritableSignal } from '@angular/core';
import type { GroupPanelPartInitParameters, IContentRenderer } from 'dockview';
import { reportFailure } from '../core/failure-reporter';
import { PANEL_VISIBLE } from '../core/panel-visibility';
import { describeError, Logger, redactPaths } from '../core/logger';

/** Id of the dock panel a view is rendered in; differs from the view id for a view opened several times (`timeline#2`). */
export const DOCK_PANEL_ID = new InjectionToken<string>('DOCK_PANEL_ID');

/**
 * Adapter that renders an Angular standalone component inside a dockview panel, behind an error boundary.
 *
 * The component is created lazily (when the panel is first visible). Its host view is attached to the
 * `ApplicationRef`, so Angular's dirty tracking refreshes only the panels whose signals changed; while the panel is
 * hidden its view is detached from change detection, so a hidden tab is never checked (it catches up when shown). The panel is
 * its own error boundary: errors in its change detection and lifecycle hooks reach `createComponent`'s `onError`
 * (Angular then carries on with the other views), errors Angular reports through the `ErrorHandler` instead
 * (effects, event handlers) reach the component's own `ErrorHandler`, and the constructor and first render run inside
 * try/catch. Any failure destroys the panel's component and shows an error card ("This panel failed: ...") with
 * Reload panel and Copy details, and is logged. Other panels keep working.
 */
export class AngularContentRenderer implements IContentRenderer {
  readonly element = document.createElement('div');
  private ref: ComponentRef<PanelOutlet> | null = null;
  private disposed = false;
  private mounted = false;
  private visibility?: { dispose(): void };
  private card: HTMLElement | null = null;
  private lastDetails = '';
  private failure: unknown = null;
  /** The component's own view (detached from change detection while the panel is hidden). */
  private view: ChangeDetectorRef | null = null;
  private attached = true;
  private readonly visible: WritableSignal<boolean> = signal(true);
  private readonly logger: Logger | null;

  constructor(
    private readonly load: () => Promise<Type<unknown>>,
    private readonly appRef: ApplicationRef,
    private readonly injector: EnvironmentInjector,
    private readonly label = 'view',
    /** The panel id; taken from dockview's init parameters when not given (a pop-out window passes it). */
    private panelId: string | null = null,
  ) {
    this.element.className = 'imt-panel';
    this.logger = injector.get(Logger, null);
  }

  /** The component is created when the panel is first visible, so hidden tabs cost nothing until opened. */
  init(params: GroupPanelPartInitParameters): void {
    const api = params.api;
    this.panelId ??= api?.id ?? null;
    this.visible.set(!api || api.isVisible);
    if (api) {
      this.visibility = api.onDidVisibilityChange((e) => this.setVisible(e.isVisible));
    }
    if (this.visible()) void this.mount();
  }

  /** Shows or hides the panel (dockview reports it; a host without dockview, such as the auto-hide flyout, calls it). */
  setVisible(visible: boolean): void {
    if (this.disposed) return;
    this.visible.set(visible);
    if (visible) void this.mount();
    this.syncAttachment();
  }

  /**
   * Includes the component in change detection while the panel is visible and leaves it out while hidden. The outlet's
   * host view stays attached to the application (detaching it would take the panel's DOM out, and with it scroll
   * positions); the outlet's own view, which holds the component and everything it declares (effects included), is
   * detached.
   */
  private syncAttachment(): void {
    const view = this.view;
    const want = this.visible();
    if (!view || want === this.attached) return;
    this.attached = want;
    if (want) view.reattach();
    else view.detach();
  }

  private async mount(): Promise<void> {
    if (this.mounted || this.disposed) return;
    this.mounted = true;
    try {
      const type = await this.load();
      if (this.disposed) return;
      const elementInjector = Injector.create({
        providers: [
          { provide: ErrorHandler, useValue: { handleError: (e: unknown) => this.fail(e) } satisfies ErrorHandler },
          { provide: PANEL_VISIBLE, useValue: this.visible.asReadonly() },
          ...(this.panelId ? [{ provide: DOCK_PANEL_ID, useValue: this.panelId }] : []),
        ],
        parent: this.injector,
      });
      const onError = (e: unknown) => this.fail(unwrapBoundaryError(e));
      const ref = createComponent(PanelOutlet, { environmentInjector: this.injector, onError });
      this.ref = ref;
      this.element.appendChild(ref.location.nativeElement as Node);
      ref.changeDetectorRef.detectChanges();
      ref.instance.outlet().createComponent(type, { environmentInjector: this.injector, injector: elementInjector, onError });
      this.view = ref.injector.get(ChangeDetectorRef);
      this.attached = true;
      // The first render, so a template that throws is caught here rather than on some later tick.
      ref.changeDetectorRef.detectChanges();
      this.appRef.attachView(ref.hostView);
      this.syncAttachment();
    } catch (e) {
      if (this.disposed) return;
      this.fail(e);
    }
  }

  /**
   * Replaces the broken component with the error card and records what happened. The first error wins; the
   * teardown is deferred so a component is never destroyed while Angular is still running its effects or hooks.
   */
  private fail(error: unknown): void {
    if (this.failure !== null || this.disposed) return;
    this.failure = error ?? new Error('unknown error');
    queueMicrotask(() => this.showFailure(this.failure));
  }

  private showFailure(error: unknown): void {
    this.destroyComponent();
    if (this.disposed) return;
    const message = describeError(error);
    const stack = error instanceof Error && error.stack ? error.stack : '';
    this.lastDetails = `Panel: ${this.label}\nError: ${message}${stack ? `\n${stack}` : ''}`;
    this.logger?.error('ui.panel', 'Panel failed', { Panel: this.label, Error: redactPaths(message), Stack: stack ? redactPaths(stack) : undefined });
    this.showCard(message);
    // After the card is in place, so an error snapshot shows it.
    reportFailure(this.injector, (r) => r.panelFailed(this.label, error));
  }

  private showCard(message: string): void {
    this.card?.remove();
    const card = document.createElement('div');
    card.className = 'imt-panel-error imt-panel-failed';
    card.setAttribute('role', 'alert');
    card.style.cssText = 'display:flex;flex-direction:column;align-items:flex-start;gap:8px;color:var(--text-2);';
    const title = document.createElement('strong');
    title.textContent = $localize`:@@dock.panel.failed:This panel failed: ${message}:message:`;
    const hint = document.createElement('span');
    hint.textContent = $localize`:@@dock.panel.stopped:The ${this.label}:label: panel stopped, the rest of the app is unaffected.`;
    const actions = document.createElement('div');
    actions.className = 'imt-panel-actions';
    actions.style.cssText = 'display:flex;gap:8px;';
    const reload = this.button($localize`:@@dock.panel.reload:Reload panel`, () => this.reload());
    const copy = this.button($localize`:@@dock.panel.copyDetails:Copy details`, () => void this.copyDetails(copy));
    actions.append(reload, copy);
    card.append(title, hint, actions);
    this.element.appendChild(card);
    this.card = card;
  }

  private button(text: string, onClick: () => void): HTMLButtonElement {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'btn';
    b.textContent = text;
    b.addEventListener('click', onClick);
    return b;
  }

  /** Tries again from scratch: a new component instance. */
  reload(): void {
    this.card?.remove();
    this.card = null;
    this.failure = null;
    this.destroyComponent();
    this.mounted = false;
    void this.mount();
  }

  private async copyDetails(button: HTMLButtonElement): Promise<void> {
    try {
      await navigator.clipboard.writeText(this.lastDetails);
      button.textContent = $localize`:@@dock.panel.copied:Copied`;
    } catch {
      button.textContent = $localize`:@@dock.panel.copyFailed:Copy failed`;
    }
    setTimeout(() => (button.textContent = $localize`:@@dock.panel.copyDetails:Copy details`), 1500);
  }

  private destroyComponent(): void {
    this.view = null;
    const ref = this.ref;
    this.ref = null;
    if (!ref) return;
    try {
      ref.destroy();
    } catch {
      /* a component that already failed may fail to tear down; the card replaces it either way */
    }
    ref.location.nativeElement.remove?.();
  }

  dispose(): void {
    this.disposed = true;
    this.visibility?.dispose();
    this.destroyComponent();
    this.element.remove();
  }
}

/**
 * Holds a panel's component, so that everything the component declares lives in a view this renderer can detach from
 * change detection while the panel is hidden (effects of a component's constructor belong to its parent view).
 */
@Component({
  selector: 'imt-panel-outlet',
  host: { style: 'display: contents' },
  template: '<ng-container #outlet />',
})
class PanelOutlet {
  readonly outlet = viewChild.required('outlet', { read: ViewContainerRef });
}

/** Angular wraps a thrown non-Error (`throw 'text'`) before handing it to an error boundary; the card shows the original. */
function unwrapBoundaryError(e: unknown): unknown {
  return e instanceof Error && e.name === 'ErrorBoundaryWrappedError' && 'cause' in e ? e.cause : e;
}

/** Placeholder for a panel in a saved layout whose view no longer exists (e.g. a removed plugin). */
export class MissingViewRenderer implements IContentRenderer {
  readonly element = document.createElement('div');
  constructor(id: string) {
    this.element.className = 'imt-panel imt-panel-error';
    this.element.textContent = $localize`:@@dock.panel.missingView:The view "${id}:id:" is not available. Close this tab or reset the layout.`;
  }
  init(): void {}
}
