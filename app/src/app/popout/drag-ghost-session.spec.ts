import { Rect } from '../dock/dock-geometry';
import { DragGhostBackend, GhostRequest } from './drag-ghost-backend';
import { GhostSession } from './drag-ghost-session';

const REQUEST: GhostRequest = { title: 'Metrics', width: 560, height: 420, grabX: 120, grabY: 12 };

class RecordingGhost extends DragGhostBackend {
  readonly available = true;
  calls: string[] = [];
  starts = true;
  failShow = false;
  at: Rect | null = { x: 10, y: 20, width: 560, height: 420 };
  async start(r: GhostRequest) {
    this.calls.push(`start:${r.title}`);
    return this.starts;
  }
  async show(visible: boolean) {
    this.calls.push(`show:${visible}`);
    if (this.failShow) throw new Error('gone');
  }
  async end() {
    this.calls.push('end');
    return this.at;
  }
}

const tick = () => new Promise((r) => setTimeout(r));

describe('GhostSession', () => {
  it('runs the shell calls in order after the start, sends only changes and ends once', async () => {
    const ghost = new RecordingGhost();
    let release!: (r: GhostRequest) => void;
    const s = new GhostSession(ghost, new Promise((r) => (release = r)));
    expect(s.native).toBe(true);
    s.show(true);
    s.show(true);
    s.show(false);
    s.show(true);
    // Nothing reaches the shell before the request (with its snapshot) is ready.
    expect(ghost.calls).toEqual([]);
    release(REQUEST);
    const end = s.finish();
    expect(s.finish()).toBe(end);
    await expect(end).resolves.toEqual({ x: 10, y: 20, width: 560, height: 420 });
    // Changes queued while starting collapse into the latest one.
    expect(ghost.calls).toEqual(['start:Metrics', 'show:true', 'end']);
    s.show(false);
    await tick();
    expect(ghost.calls.at(-1)).toBe('end');
  });

  it('turns off where the shell has no ghost, so the dock draws its own preview', async () => {
    const ghost = new RecordingGhost();
    ghost.starts = false;
    const s = new GhostSession(ghost, Promise.resolve(REQUEST));
    s.show(true);
    await tick();
    expect(s.native).toBe(false);
    await expect(s.finish()).resolves.toBeNull();
    expect(ghost.calls).toEqual(['start:Metrics']);
  });

  it('a failed request or call turns the ghost off and still ends the session in the shell', async () => {
    const ghost = new RecordingGhost();
    const s = new GhostSession(ghost, Promise.reject(new Error('no snapshot')));
    await tick();
    expect(s.native).toBe(false);
    await expect(s.finish()).resolves.toBeNull();
    expect(ghost.calls).toEqual([]);

    const failing = new RecordingGhost();
    failing.failShow = true;
    const t = new GhostSession(failing, Promise.resolve(REQUEST));
    t.show(true);
    await tick();
    await tick();
    expect(t.native).toBe(false);
    expect(failing.calls).toEqual(['start:Metrics', 'show:true', 'end']);
    await expect(t.finish()).resolves.toBeNull();
  });

  it('a null request starts nothing', async () => {
    const ghost = new RecordingGhost();
    const s = new GhostSession(ghost, Promise.resolve(null));
    await expect(s.finish()).resolves.toBeNull();
    expect(ghost.calls).toEqual([]);
  });
});
