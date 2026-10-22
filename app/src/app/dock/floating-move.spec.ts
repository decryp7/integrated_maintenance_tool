import { FloatingMove, isFloatingMovePress } from './floating-move';

function floatingHandle(): { root: HTMLElement; handle: HTMLElement; tab: HTMLElement; docked: HTMLElement } {
  const root = document.createElement('div');
  root.innerHTML = `
    <div class="dv-resize-container"><div class="dv-tabs-and-actions-container"><div class="dv-tab" id="tab"></div><div class="dv-void-container" id="handle"></div></div></div>
    <div class="dv-groupview"><div class="dv-void-container" id="docked"></div></div>`;
  document.body.appendChild(root);
  return { root, handle: root.querySelector('#handle')!, tab: root.querySelector('#tab')!, docked: root.querySelector('#docked')! };
}

const press = (over: Partial<PointerEventInit> = {}) => ({ button: 0, shiftKey: false, pointerType: 'mouse', ...over });

describe('isFloatingMovePress', () => {
  it('is a plain primary mouse press on a floating window\'s empty tab strip', () => {
    const { root, handle, tab, docked } = floatingHandle();
    expect(isFloatingMovePress(press(), handle)).toBe(true);
    expect(isFloatingMovePress(press({ shiftKey: true }), handle)).toBe(false);
    expect(isFloatingMovePress(press({ button: 2 }), handle)).toBe(false);
    expect(isFloatingMovePress(press({ pointerType: 'touch' }), handle)).toBe(false);
    expect(isFloatingMovePress(press({ pointerType: 'pen' }), handle)).toBe(false);
    expect(isFloatingMovePress(press(), tab)).toBe(false);
    expect(isFloatingMovePress(press(), docked)).toBe(false);
    expect(isFloatingMovePress(press(), null)).toBe(false);
    root.remove();
  });
});

describe('FloatingMove', () => {
  const pointer = (type: string, init: PointerEventInit) => new PointerEvent(type, { bubbles: true, pointerId: 1, pointerType: 'mouse', ...init });

  it('hands the window move one movement at the press point before the first real one', () => {
    const { root, handle } = floatingHandle();
    const move = new FloatingMove(root);
    const seen: { x: number; y: number; trusted: boolean }[] = [];
    const listen = (e: PointerEvent) => seen.push({ x: e.clientX, y: e.clientY, trusted: e === real });
    window.addEventListener('pointermove', listen);
    handle.dispatchEvent(pointer('pointerdown', { button: 0, clientX: 10, clientY: 20 }));
    let real = pointer('pointermove', { clientX: 40, clientY: 60 });
    window.dispatchEvent(real);
    expect(seen).toEqual([
      { x: 10, y: 20, trusted: false },
      { x: 40, y: 60, trusted: true },
    ]);
    // Only the first movement is primed.
    seen.length = 0;
    real = pointer('pointermove', { clientX: 50, clientY: 70 });
    window.dispatchEvent(real);
    expect(seen).toEqual([{ x: 50, y: 70, trusted: true }]);
    window.removeEventListener('pointermove', listen);
    move.dispose();
    root.remove();
  });

  it('leaves other presses, a released press and a disposed guard alone', () => {
    const { root, handle, tab } = floatingHandle();
    const move = new FloatingMove(root);
    let count = 0;
    const listen = () => count++;
    window.addEventListener('pointermove', listen);
    tab.dispatchEvent(pointer('pointerdown', { button: 0 }));
    window.dispatchEvent(pointer('pointermove', { clientX: 5 }));
    expect(count).toBe(1);
    handle.dispatchEvent(pointer('pointerdown', { button: 0, shiftKey: true }));
    window.dispatchEvent(pointer('pointermove', { clientX: 5 }));
    expect(count).toBe(2);
    handle.dispatchEvent(pointer('pointerdown', { button: 0 }));
    window.dispatchEvent(pointer('pointerup', {}));
    window.dispatchEvent(pointer('pointermove', { clientX: 5 }));
    expect(count).toBe(3);
    // Another pointer's movement is not the press's.
    handle.dispatchEvent(pointer('pointerdown', { button: 0 }));
    window.dispatchEvent(pointer('pointermove', { pointerId: 2, clientX: 5 }));
    expect(count).toBe(4);
    window.dispatchEvent(pointer('pointercancel', {}));
    move.dispose();
    handle.dispatchEvent(pointer('pointerdown', { button: 0 }));
    window.dispatchEvent(pointer('pointermove', { clientX: 5 }));
    expect(count).toBe(5);
    window.removeEventListener('pointermove', listen);
    root.remove();
  });
});
