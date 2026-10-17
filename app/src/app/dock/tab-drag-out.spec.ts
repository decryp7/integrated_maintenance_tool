import { TabDragOut } from './tab-drag-out';

describe('TabDragOut', () => {
  function setup() {
    const root = document.createElement('div');
    root.innerHTML = '<div role="tab"><div class="imt-tab" data-panel-id="metrics"><span id="label">Metrics</span></div></div><div id="body"></div>';
    document.body.appendChild(root);
    const out: string[] = [];
    const drag = new TabDragOut(root, (id, at) => out.push(`${id}@${at.x},${at.y}`), () => ({ width: 1000, height: 800 }));
    const tab = root.querySelector<HTMLElement>('[role=tab]')!;
    const p = (type: string, target: EventTarget, x: number, y: number, extra: PointerEventInit = {}) =>
      target.dispatchEvent(new PointerEvent(type, { bubbles: true, button: 0, clientX: x, clientY: y, screenX: x + 100, screenY: y + 100, pointerId: 1, ...extra }));
    return { root, drag, tab, out, p, label: root.querySelector('#label')! };
  }

  it('pops a tab out where it is released outside the window', () => {
    const { tab, out, p, label, drag, root } = setup();
    const capture = vi.fn();
    (tab as HTMLElement & { setPointerCapture: unknown }).setPointerCapture = capture;
    p('pointerdown', label, 10, 10);
    p('pointermove', window, 30, 10);
    expect(capture).toHaveBeenCalledWith(1);
    p('pointerup', window, -20, 300);
    expect(out).toEqual(['metrics@80,400']);
    drag.dispose();
    root.remove();
  });

  it('ignores clicks, other buttons, presses outside tabs, releases inside and cancelled drags', () => {
    const { out, p, label, root, drag } = setup();
    p('pointerdown', label, 10, 10);
    p('pointerup', window, 11, 10);
    p('pointerdown', label, 10, 10, { button: 2 });
    p('pointermove', window, 300, 10);
    p('pointerup', window, -20, 300);
    p('pointerdown', root.querySelector('#body')!, 10, 10);
    p('pointermove', window, 300, 10);
    p('pointerup', window, -20, 300);
    p('pointerdown', label, 10, 10);
    p('pointermove', window, 300, 10);
    p('pointerup', window, 300, 10);
    p('pointerdown', label, 10, 10);
    p('pointermove', window, 300, 10);
    p('pointercancel', window, 0, 0);
    p('pointerup', window, -20, 300);
    expect(out).toEqual([]);
    drag.dispose();
    root.remove();
  });

  it('treats a lost pointer capture as a release at the last position', () => {
    const { tab, out, p, label, drag, root } = setup();
    (tab as HTMLElement & { setPointerCapture: unknown }).setPointerCapture = () => undefined;
    p('pointerdown', label, 10, 10);
    p('pointermove', window, -30, 10);
    tab.dispatchEvent(new Event('lostpointercapture'));
    expect(out).toEqual(['metrics@70,110']);
    // A capture that cannot be taken (the pointer is gone) is not an error.
    (tab as HTMLElement & { setPointerCapture: unknown }).setPointerCapture = () => {
      throw new Error('gone');
    };
    p('pointerdown', label, 10, 10);
    p('pointermove', window, 40, 10);
    p('pointerup', window, 40, 10);
    drag.dispose();
    root.remove();
  });
});
