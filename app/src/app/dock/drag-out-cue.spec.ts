import { Rect } from './dock-geometry';
import { cueMode, DragOutCue, InWindowCue } from './drag-out-cue';
import { TabDropOutcome } from './float-cue';
import { DragOutPreview } from './panel-windows';

class FakePreview implements DragOutPreview {
  native = true;
  shown: boolean[] = [];
  finished = 0;
  constructor(private readonly where: Rect | null = { x: 1, y: 2, width: 300, height: 200 }) {}
  show(outside: boolean): void {
    this.shown.push(outside);
  }
  finish(): Promise<Rect | null> {
    this.finished++;
    return Promise.resolve(this.where);
  }
}

class FakeCue implements InWindowCue {
  log: string[] = [];
  show(outcome: TabDropOutcome, _rect: Rect, drawn = true): void {
    this.log.push(`${outcome}:${drawn ? 'drawn' : 'announced'}`);
  }
  hide(): void {
    this.log.push('hide');
  }
}

describe('cueMode', () => {
  it('outside the window the native ghost replaces the in-window preview; inside only the cancel hint is drawn', () => {
    expect(cueMode('window', true)).toEqual({ ghost: true, drawn: false });
    expect(cueMode('window', false)).toEqual({ ghost: false, drawn: true });
    expect(cueMode('cancel', true)).toEqual({ ghost: false, drawn: true });
    expect(cueMode('dock', true)).toEqual({ ghost: false, drawn: false });
    expect(cueMode(null, true)).toEqual({ ghost: false, drawn: false });
  });
});

describe('DragOutCue', () => {
  const rect = { x: 0, y: 0, width: 10, height: 10 };

  function setup(preview: FakePreview | null = new FakePreview()) {
    const cue = new FakeCue();
    const begun: string[] = [];
    const drag = new DragOutCue(cue, {
      cueRect: (_id, _p, outcome) => (outcome === 'dock' ? null : rect),
      beginDragOut: (id) => {
        begun.push(id);
        return preview;
      },
    });
    return { cue, drag, begun, preview };
  }

  it('begins the ghost with the drag, shows it outside, hides it back inside, and keeps where it was for the release', async () => {
    const { cue, drag, begun, preview } = setup();
    drag.update('metrics', { x: 5, y: 5 }, 'cancel');
    drag.update('metrics', { x: -5, y: 5 }, 'window');
    drag.update('metrics', { x: 5, y: 5 }, 'dock');
    drag.update('metrics', { x: -9, y: 5 }, 'window');
    expect(begun).toEqual(['metrics']);
    expect(preview!.shown).toEqual([false, true, false, true]);
    // The in-window preview is not drawn while the ghost shows the new window; its hint is still announced.
    expect(cue.log).toEqual(['cancel:drawn', 'window:announced', 'hide', 'window:announced']);
    drag.update('metrics', { x: -9, y: 5 }, null);
    expect(preview!.finished).toBe(1);
    expect(cue.log.at(-1)).toBe('hide');
    await expect(drag.takeEnd()).resolves.toEqual({ x: 1, y: 2, width: 300, height: 200 });
    expect(drag.takeEnd()).toBeNull();
  });

  it('draws the in-window preview when there is no ghost, or once it turned out the shell has none', () => {
    const { cue, drag } = setup(null);
    drag.update('metrics', { x: -5, y: 5 }, 'window');
    expect(cue.log).toEqual(['window:drawn']);
    drag.update('metrics', { x: -5, y: 5 }, null);
    expect(drag.takeEnd()).toBeNull();
    const off = new FakePreview();
    off.native = false;
    const second = setup(off);
    second.drag.update('events', { x: -5, y: 5 }, 'window');
    expect(second.cue.log).toEqual(['window:drawn']);
    expect(off.shown).toEqual([false]);
  });

  it('a new drag ends the previous one, and disposing ends the current one', () => {
    const first = new FakePreview();
    const { drag } = setup(first);
    drag.update('metrics', { x: 1, y: 1 }, 'cancel');
    drag.update('events', { x: 1, y: 1 }, 'cancel');
    expect(first.finished).toBe(1);
    drag.dispose();
    expect(first.finished).toBe(2);
    expect(drag.takeEnd()).toBeNull();
  });
});
