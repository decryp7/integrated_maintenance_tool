import {
  afterNextRender,
  ApplicationRef,
  ChangeDetectionStrategy,
  Component,
  computed,
  DestroyRef,
  effect,
  ElementRef,
  EnvironmentInjector,
  inject,
  Injector,
  signal,
  untracked,
  viewChild,
} from '@angular/core';
import { Logger } from '../core/logger';
import { WorkspaceStore } from '../core/workspace.store';
import { AngularContentRenderer, MissingViewRenderer } from '../dock/angular-panel';
import { DockService } from '../dock/dock.service';
import { CommandRegistry } from '../extensions/command-registry';
import { matchesKeybinding } from '../extensions/keybinding';
import { ViewRegistry } from '../extensions/view-registry';
import { CommandPalette } from '../palette/command-palette';
import { isTextEntry } from '../shell/title-bar';
import { ContextMenuHost } from '../ui/context-menu-host';
import { DialogHost } from '../ui/dialog-host';
import { errorMessage } from '../tools/tool-errors';
import { PopoutBackend } from './popout-backend';
import { POPOUT_ROUTE } from './popout-route';
import { PopoutState } from './popout-state';
import { PopoutTitleBar } from './popout-title-bar';
import { hostTitle, runsInHost } from './popout.service';
import { sharedUiState } from './shared-state';
import { UiSync } from './ui-sync';

/**
 * Keeps `document.title` at `title()` (re-applied when anything else changes it). Returns the cleanup.
 */
export function pinDocumentTitle(title: () => string, injector: Injector): () => void {
  let applying = false;
  const apply = () => {
    const want = title();
    if (document.title === want) return;
    applying = true;
    document.title = want;
    applying = false;
  };
  const ref = effect(() => apply(), { injector, manualCleanup: true });
  const observer = new MutationObserver(() => {
    if (!applying) apply();
  });
  const head = document.head ?? document.documentElement;
  observer.observe(head, { subtree: true, childList: true, characterData: true });
  return () => {
    ref.destroy();
    observer.disconnect();
  };
}

/** How often a pop-out checks that its host still exists (browser build: the opener). */
export const HOST_CHECK_MS = 1000;
/** A pop-out starts syncing once its workspace loaded, or after this long anyway. */
export const SYNC_START_TIMEOUT_MS = 5000;

/**
 * The minimal app a pop-out window boots into: a title bar (panel title, host project, Dock back, Always on top)
 * and the one view. It shares the host's interaction state through `UiSync`, queries the engine itself (commands
 * resolve to the host's workspace in Rust), and closes itself when the host goes away.
 */
@Component({
  selector: 'imt-popout-shell',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CommandPalette, ContextMenuHost, DialogHost, PopoutTitleBar],
  host: { '(document:keydown)': 'onKey($event)' },
  template: `
    <imt-popout-title-bar [title]="title()" [host]="state.hostTitle()" [onTop]="state.alwaysOnTop()" (dockBack)="dockBack()" (toggleOnTop)="toggleOnTop()" />
    <main #viewHost class="view" [attr.aria-label]="title()" [attr.aria-busy]="!mounted()">
      <p class="connecting" i18n="@@popout.shell.connecting">Connecting to the main window…</p>
    </main>
    @if (store.error(); as error) {
      <div class="toast" role="alert">
        <span>{{ error }}</span>
        <button type="button" class="btn" (click)="store.dismissError()" i18n="@@popout.shell.dismiss">Dismiss</button>
      </div>
    }
    <imt-command-palette />
    <imt-dialog-host />
    <imt-context-menu-host />
  `,
  styles: `
    :host { display: flex; flex-direction: column; height: 100vh; background: var(--bg-1); color: var(--text); }
    .view { flex: 1; min-height: 0; display: flex; }
    .view > .imt-panel { flex: 1; }
    .connecting { margin: auto; color: var(--muted); font-size: 13px; }
    .toast {
      position: fixed; inset-inline-end: 12px; bottom: 12px; max-width: 420px; z-index: 100; display: flex; align-items: center; gap: 10px;
      padding: 8px 8px 8px 12px; border-radius: 8px; background: var(--tooltip-bg); border: 1px solid var(--lvl-error); box-shadow: var(--shadow);
    }
  `,
})
export class PopoutShell {
  protected readonly store = inject(WorkspaceStore);
  protected readonly state = inject(PopoutState);
  private readonly route = inject(POPOUT_ROUTE)!;
  private readonly views = inject(ViewRegistry);
  private readonly backend = inject(PopoutBackend);
  private readonly sync = inject(UiSync);
  private readonly dock = inject(DockService);
  private readonly injector = inject(Injector);
  private readonly environment = inject(EnvironmentInjector);
  private readonly appRef = inject(ApplicationRef);
  private readonly logger = inject(Logger);
  private readonly viewHost = viewChild.required<ElementRef<HTMLElement>>('viewHost');
  private renderer: AngularContentRenderer | MissingViewRenderer | null = null;

  protected readonly mounted = signal(false);
  protected readonly title = computed(() => this.views.find(this.route.view)?.title ?? this.route.view);

  constructor() {
    const destroy = inject(DestroyRef);
    // Views asked for from here (a "show in grid" button) open in the host's layout.
    this.dock.setRemote((id) => {
      if (id !== this.route.view) this.sync.runInHost('showView', id);
    });
    // The window title is the panel's (the project service, which sets it in app windows, must not win here).
    destroy.onDestroy(pinDocumentTitle(() => `${this.title()} \u2014 ${this.state.hostTitle() || 'IMT'}`, this.injector));
    // The view appears once this window caught up with the host (or after a timeout, unsynced), so what the
    // user does in it is never overwritten by the host's snapshot.
    const rendered = new Promise<void>((resolve) => afterNextRender(() => resolve()));
    const synced = new Promise<void>((resolve) => {
      this.sync.onSynced(resolve);
      const t = setTimeout(resolve, SYNC_START_TIMEOUT_MS * 2);
      destroy.onDestroy(() => clearTimeout(t));
    });
    void Promise.all([rendered, synced]).then(() => this.mountView());
    this.startSyncWhenLoaded(destroy);
    const timer = setInterval(() => {
      if (this.backend.hostGone()) this.closeSelf('host gone');
    }, HOST_CHECK_MS);
    this.sync.onHostGone(() => this.closeSelf('host said bye'));
    // The shared native menu (macOS) sends its clicks to the focused window, which may be this one.
    let offMenu: (() => void) | null = null;
    void this.backend.onMenu((id) => this.runCommand(id)).then((u) => (offMenu = u), () => undefined);
    destroy.onDestroy(() => offMenu?.());
    destroy.onDestroy(() => {
      clearInterval(timer);
      (this.renderer as { dispose?(): void } | null)?.dispose?.();
      this.dock.setRemote(null);
    });
  }

  /** The view behind the panel's error boundary (one crashing view never takes the window down). */
  private mountView(): void {
    if (this.mounted()) return;
    this.mounted.set(true);
    const view = this.views.find(this.route.view);
    this.renderer = view ? new AngularContentRenderer(view.component, this.appRef, this.environment, view.title, this.route.view) : new MissingViewRenderer(this.route.view);
    this.renderer.init({} as never);
    this.viewHost().nativeElement.replaceChildren(this.renderer.element);
  }

  /**
   * Sync starts after this window loaded its workspace summary: loading resets the selection, which must not be
   * sent over the host's. Then the host's snapshot (hello) brings this window up to date.
   */
  private startSyncWhenLoaded(destroy: DestroyRef): void {
    let started = false;
    const start = () => {
      if (started) return;
      started = true;
      const port = this.backend.syncPort(this.route.host, false, () => [this.route.host]);
      this.sync.start(port, false, this.route.label, [...sharedUiState(this.injector), hostTitle(this.injector)]);
    };
    const fallback = setTimeout(start, SYNC_START_TIMEOUT_MS);
    const ref = effect(
      () => {
        if (this.store.summary() && !this.store.busy()) untracked(start);
      },
      { injector: this.injector },
    );
    destroy.onDestroy(() => {
      clearTimeout(fallback);
      ref.destroy();
    });
  }

  protected async dockBack(): Promise<void> {
    try {
      await this.backend.dockSelf(this.route);
    } catch (e) {
      this.store.setError($localize`:@@popout.shell.dockBackFailed:Could not dock the panel back: ${errorMessage(e)}:error:`);
    }
  }

  protected async toggleOnTop(): Promise<void> {
    const on = !this.state.alwaysOnTop();
    try {
      await this.backend.setAlwaysOnTop(on);
      this.state.alwaysOnTop.set(on);
    } catch (e) {
      this.store.setError($localize`:@@popout.shell.onTopFailed:Could not keep the window on top: ${errorMessage(e)}:error:`);
    }
  }

  /** Keys: commands on the main window (undo/redo, layout, panels) run in the host; the rest here (palette, view commands). */
  protected onKey(e: KeyboardEvent): void {
    const target = e.target as HTMLElement | null;
    if (isTextEntry(target) && (e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === 'z') return;
    const registry = this.injector.get(CommandRegistry);
    const command = registry.available().find((c) => c.keybinding && matchesKeybinding(c.keybinding, e));
    if (!command) return;
    e.preventDefault();
    this.runCommand(command.id);
  }

  /** Runs a command for this window: here, or in the host for what acts on the main window (layout, history). */
  private runCommand(id: string): void {
    if (runsInHost(id)) this.sync.runInHost('run', id);
    else void this.injector.get(CommandRegistry).run(id);
  }

  private closeSelf(reason: string): void {
    this.logger.info('ui.popout', 'Pop-out closes itself', { Reason: reason });
    window.close();
  }
}
