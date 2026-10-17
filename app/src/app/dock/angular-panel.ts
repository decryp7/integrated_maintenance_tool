import { AfterRenderRef, afterEveryRender, ApplicationRef, ComponentRef, createComponent, EnvironmentInjector, ErrorHandler, Injector, Type } from '@angular/core';
import type { GroupPanelPartInitParameters, IContentRenderer } from 'dockview';
import { reportFailure } from '../core/failure-reporter';
import { describeError, Logger, redactPaths } from '../core/logger';

/**
 * Adapter that renders an Angular standalone component inside a dockview panel, behind an error boundary.
 *
 * The component is created lazily (when the panel is first visible). It is deliberately *not* attached to the
 * `ApplicationRef`: Angular has no per-view error handling, so one view that throws while rendering would abort
 * every later view's refresh on each tick and take the whole layout down. Instead this renderer refreshes the
 * panel itself after each application render (`afterEveryRender`), inside try/catch. Errors Angular reports
 * through the `ErrorHandler` instead of throwing (effects, event handlers) are routed to the panel too, by giving
 * the component its own `ErrorHandler`. Any failure - the
 * constructor, the first render, a later render or an effect - destroys the panel's component and shows an error
 * card ("This panel failed: ...") with Reload panel and Copy details, and is logged. Other panels keep working.
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
  private card: HTMLElement | null = null;
  private lastDetails = '';
  private failure: unknown = null;
  private hook: AfterRenderRef | null = null;
  private readonly logger: Logger | null;

  /** `_appRef` is unused (kept so callers do not change): panels are refreshed by this renderer, not by the application. */
  constructor(
    private readonly load: () => Promise<Type<unknown>>,
    _appRef: ApplicationRef,
    private readonly injector: EnvironmentInjector,
    private readonly label = 'view',
  ) {
    this.element.className = 'imt-panel';
    this.logger = injector.get(Logger, null);
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
    let ref: ComponentRef<unknown> | null = null;
    try {
      const type = await this.load();
      if (this.disposed) return;
      const elementInjector = Injector.create({
        providers: [{ provide: ErrorHandler, useValue: { handleError: (e: unknown) => this.fail(e) } satisfies ErrorHandler }],
        parent: this.injector,
      });
      ref = createComponent(type, { environmentInjector: this.injector, elementInjector });
      this.ref = ref;
      this.element.appendChild(ref.location.nativeElement as Node);
      // The first render, so a template that throws is caught here rather than on some later tick.
      ref.changeDetectorRef.detectChanges();
      const mine = ref;
      // Registered on the environment injector: hooks of a component's own injector only run while its view is attached.
      this.hook = afterEveryRender(
        () => {
          if (this.ref !== mine || this.disposed || this.failure !== null) return;
          try {
            mine.changeDetectorRef.detectChanges();
          } catch (e) {
            this.fail(e);
          }
        },
        { injector: this.injector },
      );
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
    title.textContent = `This panel failed: ${message}`;
    const hint = document.createElement('span');
    hint.textContent = `The ${this.label} panel stopped, the rest of the app is unaffected.`;
    const actions = document.createElement('div');
    actions.className = 'imt-panel-actions';
    actions.style.cssText = 'display:flex;gap:8px;';
    const reload = this.button('Reload panel', () => this.reload());
    const copy = this.button('Copy details', () => void this.copyDetails(copy));
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
      button.textContent = 'Copied';
    } catch {
      button.textContent = 'Copy failed';
    }
    setTimeout(() => (button.textContent = 'Copy details'), 1500);
  }

  private destroyComponent(): void {
    this.hook?.destroy();
    this.hook = null;
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

/** Placeholder for a panel in a saved layout whose view no longer exists (e.g. a removed plugin). */
export class MissingViewRenderer implements IContentRenderer {
  readonly element = document.createElement('div');
  constructor(id: string) {
    this.element.className = 'imt-panel imt-panel-error';
    this.element.textContent = `The view "${id}" is not available. Close this tab or reset the layout.`;
  }
  init(): void {}
}
