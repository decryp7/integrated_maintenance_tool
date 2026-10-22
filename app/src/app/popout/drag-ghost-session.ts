import { Rect } from '../dock/dock-geometry';
import { DragOutPreview } from '../dock/panel-windows';
import { DragGhostBackend, GhostRequest } from './drag-ghost-backend';

/**
 * One drag's ghost window: started when the drag begins (the request, with its snapshot, may take a moment), shown
 * and hidden as the pointer leaves and re-enters the window, ended with the drag. The shell's calls run in order;
 * a failure anywhere turns the ghost off (`native` false) so the dock draws its in-window preview instead, and the
 * session is always ended in the shell so the ghost never stays on screen.
 */
export class GhostSession implements DragOutPreview {
  private phase: 'starting' | 'on' | 'off' = 'starting';
  private chain: Promise<unknown>;
  private wanted = false;
  private sent = false;
  private result: Promise<Rect | null> | null = null;

  constructor(
    private readonly backend: DragGhostBackend,
    request: Promise<GhostRequest | null>,
  ) {
    this.chain = request
      .then((r) => (r ? backend.start(r) : false))
      .then(
        (on) => (this.phase = on ? 'on' : 'off'),
        () => (this.phase = 'off'),
      );
  }

  get native(): boolean {
    return this.phase !== 'off';
  }

  show(outside: boolean): void {
    if (this.result || outside === this.wanted) return;
    this.wanted = outside;
    this.then(async () => {
      if (this.phase !== 'on' || this.sent === this.wanted) return;
      this.sent = this.wanted;
      await this.backend.show(this.wanted);
    });
  }

  finish(): Promise<Rect | null> {
    this.result ??= this.then(async () => (this.phase === 'on' ? this.backend.end() : null)).then((r) => r ?? null);
    return this.result;
  }

  /** Runs `step` after the calls before it; a failing call turns the ghost off (and ends nothing more). */
  private then<T>(step: () => Promise<T>): Promise<T | null> {
    const next = this.chain.then(step).catch(() => {
      if (this.phase === 'on') void this.backend.end().catch(() => undefined);
      this.phase = 'off';
      return null;
    });
    this.chain = next;
    return next;
  }
}
