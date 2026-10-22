import { computed, DestroyRef, inject, Injectable, Injector, signal } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { CommandRegistry } from '../extensions/command-registry';
import { errorMessage } from '../tools/tool-errors';
import { LiveTail } from '../workspace/live-tail';
import { ClosePromptService } from './close-prompt';
import { OpenAskService } from './open-ask-dialog';
import { Unlisten } from './window-events';
import { WindowingBackend } from './windowing-backend';
import { BackgroundTask, WindowInfo } from './windowing.types';

/** Command run when the tray asks for a diagnostic screenshot. */
export const CAPTURE_COMMAND = 'diagnostics.capture';

/**
 * This window's view of the app's windows and background work, and the reactions to the shell's
 * window events (close prompt, open prompt, capture, live tail stopped from the tray).
 */
@Injectable({ providedIn: 'root' })
export class WindowingService {
  private readonly backend = inject(WindowingBackend);
  private readonly store = inject(WorkspaceStore);
  private readonly closePrompt = inject(ClosePromptService);
  private readonly openAsk = inject(OpenAskService);
  // Resolved lazily: the command registry's contributions include this service's commands.
  private readonly injector = inject(Injector);
  private readonly unlisten: Unlisten[] = [];
  private started: Promise<void> | null = null;

  /** This window's label (empty until `init` has asked the shell). */
  readonly label = signal('');
  readonly windows = signal<readonly WindowInfo[]>([]);
  readonly tasks = signal<readonly BackgroundTask[]>([]);
  readonly runningCount = computed(() => this.tasks().length);

  constructor() {
    inject(DestroyRef).onDestroy(() => {
      for (const u of this.unlisten.splice(0)) u();
    });
  }

  /** Subscribes to the shell's events and loads windows and tasks. Idempotent. */
  init(): Promise<void> {
    this.started ??= this.start();
    return this.started;
  }

  async newWindow(): Promise<void> {
    await this.attempt((error) => $localize`:@@windowing.service.newFailed:Could not open a new window: ${error}:error:`, async () => {
      await this.backend.newWindow();
      await this.refreshWindows();
    });
  }

  async closeWindow(): Promise<void> {
    await this.attempt((error) => $localize`:@@windowing.service.closeFailed:Could not close the window: ${error}:error:`, () => this.backend.closeWindow());
  }

  /** Quits Debrief (every window). The caller asks about unsaved work first. */
  async quitApp(): Promise<void> {
    await this.attempt((error) => $localize`:@@windowing.service.quitFailed:Could not quit Debrief: ${error}:error:`, () => this.backend.quitApp());
  }

  async focus(label: string): Promise<void> {
    await this.attempt((error) => $localize`:@@windowing.service.switchFailed:Could not switch windows: ${error}:error:`, () => this.backend.focusWindow(label));
  }

  /** Focuses the window after this one (wrapping around); nothing when this is the only window. */
  async focusNext(): Promise<void> {
    const list = this.windows();
    if (list.length < 2) return;
    const i = list.findIndex((w) => w.label === this.label());
    const next = list[(i + 1) % list.length];
    if (next) await this.focus(next.label);
  }

  async stopTask(id: string): Promise<void> {
    await this.attempt((error) => $localize`:@@windowing.service.stopFailed:Could not stop the background work: ${error}:error:`, async () => {
      await this.backend.stopBackgroundTask(id);
      this.tasks.update((t) => t.filter((x) => x.id !== id));
    });
  }

  private async start(): Promise<void> {
    await this.backend.label().then((l) => this.label.set(l), () => undefined);
    const subscriptions = await Promise.allSettled([
      this.backend.onWindowsChanged((w) => this.windows.set(w)),
      this.backend.onBackgroundTasks((t) => this.tasks.set(t)),
      this.backend.onCloseRequested(() => void this.closePrompt.open()),
      this.backend.onOpenAsk((ask) => void this.openAsk.open(ask)),
      this.backend.onCapture(() => void this.injector.get(CommandRegistry).run(CAPTURE_COMMAND)),
      this.backend.onLiveStopped(() => void this.injector.get(LiveTail).set(false)),
    ]);
    for (const s of subscriptions) if (s.status === 'fulfilled') this.unlisten.push(s.value);
    await Promise.allSettled([this.refreshWindows(), this.backend.backgroundTasks().then((t) => this.tasks.set(t))]);
  }

  private async refreshWindows(): Promise<void> {
    this.windows.set(await this.backend.listWindows());
  }

  private async attempt(failure: (error: string) => string, op: () => Promise<void>): Promise<void> {
    try {
      await op();
    } catch (e) {
      this.store.setError(failure(errorMessage(e)));
    }
  }
}
