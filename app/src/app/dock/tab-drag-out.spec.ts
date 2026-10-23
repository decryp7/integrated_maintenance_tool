import { outsideViewport } from './dock-target';
import { TabDropOutcome } from './float-cue';
import { TabDragOut } from './tab-drag-out';

describe('TabDragOut', () => {
  function setup() {
    const root = document.createElement('div');
    root.innerHTML = '<div role="tab"><div class="debrief-tab" data-panel-id="metrics"><span id="label">Metrics</span></div></div><div id="body"></div>';
    document.body.appendChild(root);
    const out: string[] = [];
    const cues: string[] = [];
    // A dock whose drop zone is the band y < 100; elsewhere inside it nothing happens, outside it opens a window.
    const outcome = (_: string, p: { x: number; y: number }): TabDropOutcome =>
      outsideViewport(p, { width: 1000, height: 800 }) ? 'window' : p.y < 100 ? 'dock' : 'cancel';
    const drag = new TabDragOut(root, {
      outcome,
      popOut: (id, at) => out.push(`pop ${id}@${at.x},${at.y}`),
      cue: (id, p, o) => cues.push(`${o ?? 'clear'}@${p.x},${p.y}`),
    });
    const tab = root.querySelector<HTMLElement>('[role=tab]')!;
    const p = (type: string, target: EventTarget, x: number, y: number, extra: PointerEventInit = {}) =>
      target.dispatchEvent(new PointerEvent(type, { bubbles: true, button: 0, clientX: x, clientY: y, screenX: x + 100, screenY: y + 100, pointerId: 1, ...extra }));
    const done = () => {
      drag.dispose();
      root.remove();
    };
    return { root, drag, tab, out, cues, p, label: root.querySelector('#label')!, done };
  }

  it('pops a tab out where it is released outside the window', () => {
    const { tab, out, p, label, done } = setup();
    const capture = vi.fn();
    (tab as HTMLElement & { setPointerCapture: unknown }).setPointerCapture = capture;
    p('pointerdown', label, 10, 10);
    p('pointermove', window, 30, 10);
    expect(capture).toHaveBeenCalledWith(1);
    p('pointerup', window, -20, 300);
    expect(out).toEqual(['pop metrics@80,400']);
    done();
  });

  it('does nothing for a tab released inside the window, on or off the drop zones (dockview docks on a zone)', () => {
    const { out, p, label, done } = setup();
    p('pointerdown', label, 10, 10);
    p('pointermove', window, 300, 400);
    p('pointerup', window, 300, 400);
    p('pointerdown', label, 10, 10);
    p('pointermove', window, 300, 50);
    p('pointerup', window, 300, 50);
    expect(out).toEqual([]);
    done();
  });

  it('shows the cue while dragging, follows the outcome and clears it on release', () => {
    const { cues, p, label, done } = setup();
    p('pointerdown', label, 10, 10);
    p('pointermove', window, 12, 10);
    expect(cues).toEqual([]);
    p('pointermove', window, 300, 400);
    p('pointermove', window, 300, 50);
    p('pointermove', window, -10, 50);
    p('pointerup', window, -10, 50);
    expect(cues).toEqual(['cancel@300,400', 'dock@300,50', 'window@-10,50', 'clear@-10,50']);
    done();
  });

  it('Escape cancels the drag: the cue clears, nothing happens on release, and dockview gets a pointercancel', () => {
    const { out, cues, p, label, done } = setup();
    const cancelled = vi.fn();
    window.addEventListener('pointercancel', cancelled);
    p('pointerdown', label, 10, 10);
    p('pointermove', window, 300, 400);
    const esc = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    window.dispatchEvent(esc);
    expect(esc.defaultPrevented).toBe(true);
    expect(cancelled).toHaveBeenCalledTimes(1);
    p('pointerup', window, 300, 400);
    expect(out).toEqual([]);
    expect(cues.at(-1)).toBe('clear@300,400');
    // Escape without a drag is left alone.
    const idle = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    window.dispatchEvent(idle);
    expect(idle.defaultPrevented).toBe(false);
    window.removeEventListener('pointercancel', cancelled);
    done();
  });

  it('ignores clicks, other buttons, presses outside tabs and cancelled drags', () => {
    const { out, cues, p, label, root, done } = setup();
    p('pointerdown', label, 10, 10);
    p('pointerup', window, 11, 10);
    p('pointerdown', label, 10, 10, { button: 2 });
    p('pointermove', window, 300, 10);
    p('pointerup', window, -20, 300);
    p('pointerdown', root.querySelector('#body')!, 10, 10);
    p('pointermove', window, 300, 10);
    p('pointerup', window, -20, 300);
    p('pointerdown', label, 10, 10);
    p('pointermove', window, 300, 400);
    p('pointercancel', window, 0, 0);
    p('pointerup', window, -20, 300);
    expect(out).toEqual([]);
    expect(cues).toEqual(['cancel@300,400', 'clear@300,400']);
    done();
  });

  it('treats a lost pointer capture as a release at the last position', () => {
    const { tab, out, p, label, done } = setup();
    (tab as HTMLElement & { setPointerCapture: unknown }).setPointerCapture = () => undefined;
    p('pointerdown', label, 10, 10);
    p('pointermove', window, -30, 10);
    tab.dispatchEvent(new Event('lostpointercapture'));
    expect(out).toEqual(['pop metrics@70,110']);
    // A capture that cannot be taken (the pointer is gone) is not an error.
    (tab as HTMLElement & { setPointerCapture: unknown }).setPointerCapture = () => {
      throw new Error('gone');
    };
    p('pointerdown', label, 10, 10);
    p('pointermove', window, 40, 10);
    p('pointerup', window, 40, 10);
    done();
  });
});
