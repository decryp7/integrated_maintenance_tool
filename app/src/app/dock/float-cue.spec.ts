import { floatedSize, FLOAT_ORIGIN, cancelHintRect, cueHint, DRAG_CANCEL_CLASS, FLOAT_MAX, FLOAT_MIN, FloatCue, ghostRect, previewSize, tabDropOutcome } from './float-cue';

describe('floatedSize', () => {
  it('is roomy on a big window, capped, and stays inside a small one', () => {
    expect(floatedSize({ width: 1440, height: 900 })).toEqual({ width: 864, height: 630 });
    expect(floatedSize({ width: 3000, height: 2000 })).toEqual({ width: 960, height: 640 });
    const small = floatedSize({ width: 600, height: 400 });
    expect(FLOAT_ORIGIN.x + small.width).toBeLessThanOrEqual(600);
    expect(FLOAT_ORIGIN.y + small.height).toBeLessThanOrEqual(400);
  });
});

describe('float cue geometry', () => {
  it('decides what a release does', () => {
    const at = { inside: true, overDropZone: false, canPopOut: true };
    // Inside the window but off every drop guide the drag is cancelled: a drag never floats a panel.
    expect(tabDropOutcome(at)).toBe('cancel');
    expect(tabDropOutcome({ ...at, canPopOut: false })).toBe('cancel');
    expect(tabDropOutcome({ ...at, overDropZone: true })).toBe('dock');
    expect(tabDropOutcome({ ...at, inside: false })).toBe('window');
    expect(tabDropOutcome({ ...at, inside: false, canPopOut: false })).toBe('cancel');
    // Outside the window, drop zones do not matter.
    expect(tabDropOutcome({ inside: false, overDropZone: true, canPopOut: true })).toBe('window');
  });

  it('previews a new window at the panel size, at most half the window and at least the minimum', () => {
    const viewport = { width: 1400, height: 900 };
    expect(previewSize({ width: 500, height: 300 }, viewport)).toEqual({ width: 500, height: 300 });
    expect(previewSize({ width: 1300, height: 800 }, viewport)).toEqual({ width: 700, height: 450 });
    expect(previewSize({ width: 20, height: 10 }, viewport)).toEqual(FLOAT_MIN);
    expect(previewSize({ width: 300.4, height: 200.6 }, viewport)).toEqual({ width: 300, height: 201 });
    expect(previewSize(null, viewport)).toEqual(FLOAT_MAX);
  });

  it('holds the ghost near its top-left like a title bar', () => {
    const bounds = { x: 0, y: 0, width: 1200, height: 800 };
    expect(ghostRect({ x: 500, y: 300 }, { width: 400, height: 300 }, bounds)).toEqual({ x: 380, y: 288, width: 400, height: 300 });
    // A narrow ghost is held by its middle.
    expect(ghostRect({ x: 500, y: 300 }, { width: 100, height: 100 }, bounds)).toEqual({ x: 450, y: 288, width: 100, height: 100 });
  });

  it('clamps the ghost into the bounds, also when the pointer is outside them', () => {
    const bounds = { x: 48, y: 30, width: 1000, height: 700 };
    const size = { width: 400, height: 300 };
    expect(ghostRect({ x: 60, y: 35 }, size, bounds)).toEqual({ x: 48, y: 30, width: 400, height: 300 });
    expect(ghostRect({ x: 1040, y: 720 }, size, bounds)).toEqual({ x: 648, y: 430, width: 400, height: 300 });
    expect(ghostRect({ x: -200, y: -50 }, size, bounds)).toEqual({ x: 48, y: 30, width: 400, height: 300 });
    expect(ghostRect({ x: 5000, y: 5000 }, size, bounds)).toEqual({ x: 648, y: 430, width: 400, height: 300 });
  });

  it('shrinks a ghost bigger than the bounds and survives empty bounds', () => {
    expect(ghostRect({ x: 100, y: 100 }, { width: 900, height: 900 }, { x: 0, y: 0, width: 600, height: 400 })).toEqual({ x: 0, y: 0, width: 600, height: 400 });
    expect(ghostRect({ x: 10, y: 10 }, { width: 400, height: 300 }, { x: 0, y: 0, width: 0, height: 0 })).toEqual({ x: 0, y: 0, width: 0, height: 0 });
  });

  it('puts the cancel hint right of and below the pointer, under the dragged tab chip', () => {
    expect(cancelHintRect({ x: 100, y: 50 })).toEqual({ x: 114, y: 96, width: 0, height: 0 });
  });

  it('has a hint for new windows and cancelling, none on a drop guide', () => {
    expect(cueHint('cancel')).toBe('Release to cancel');
    expect(cueHint('window')).toBe('Release to open in a new window');
    expect(cueHint('dock')).toBeNull();
  });
});

describe('FloatCue', () => {
  let cue: FloatCue;
  beforeEach(() => (cue = new FloatCue()));
  afterEach(() => cue.dispose());

  const ghost = () => document.querySelector<HTMLElement>('.debrief-float-cue')!;
  const live = () => document.querySelector<HTMLElement>('[aria-live="polite"].sr-only')!;
  const rect = { x: 10, y: 20, width: 300, height: 200 };

  it('starts hidden, with a polite live region and a ghost hidden from assistive technology', () => {
    expect(ghost().hidden).toBe(true);
    expect(ghost().getAttribute('aria-hidden')).toBe('true');
    expect(live().textContent).toBe('');
    expect(cue.shown).toBeNull();
  });

  it('draws the ghost with the hint and announces it once per change', () => {
    cue.show('window', rect);
    expect(ghost().hidden).toBe(false);
    expect(ghost().dataset['outcome']).toBe('window');
    expect(ghost().textContent).toBe('Release to open in a new window');
    expect([ghost().style.left, ghost().style.top, ghost().style.width, ghost().style.height]).toEqual(['10px', '20px', '300px', '200px']);
    expect(live().textContent).toBe('Release to open in a new window');
    cue.show('window', { ...rect, x: 40 });
    expect(ghost().style.left).toBe('40px');
    expect(cue.shown).toBe('window');
    expect(document.body.classList.contains(DRAG_CANCEL_CLASS)).toBe(false);
    // Off the guides inside the window: the cancel hint, and the dragged tab chip dims.
    cue.show('cancel', rect);
    expect(ghost().dataset['outcome']).toBe('cancel');
    expect(ghost().textContent).toBe('Release to cancel');
    expect(live().textContent).toBe('Release to cancel');
    expect(document.body.classList.contains(DRAG_CANCEL_CLASS)).toBe(true);
    cue.show('window', rect);
    expect(document.body.classList.contains(DRAG_CANCEL_CLASS)).toBe(false);
    cue.show('cancel', rect);
    cue.hide();
    expect(document.body.classList.contains(DRAG_CANCEL_CLASS)).toBe(false);
  });

  it('clears on a drop zone, on hide and when disposed', () => {
    cue.show('window', rect);
    cue.show('dock', rect);
    expect(ghost().hidden).toBe(true);
    expect(live().textContent).toBe('');
    expect(cue.shown).toBeNull();
    cue.show('window', rect);
    cue.hide();
    cue.hide();
    expect(ghost().hidden).toBe(true);
    expect(ghost().dataset['outcome']).toBeUndefined();
    cue.dispose();
    expect(document.querySelector('.debrief-float-cue')).toBeNull();
    cue = new FloatCue();
  });
});
