import { cueHint, FLOAT_MAX, FLOAT_MIN, FloatCue, floatSize, ghostRect, tabDropOutcome } from './float-cue';

describe('float cue geometry', () => {
  it('decides what a release does', () => {
    const at = { inside: true, overDropZone: false, canFloat: true, canPopOut: true };
    expect(tabDropOutcome(at)).toBe('float');
    expect(tabDropOutcome({ ...at, overDropZone: true })).toBe('dock');
    expect(tabDropOutcome({ ...at, canFloat: false })).toBe('none');
    expect(tabDropOutcome({ ...at, inside: false })).toBe('window');
    expect(tabDropOutcome({ ...at, inside: false, canPopOut: false })).toBe('none');
    // Outside the window, drop zones and floating do not matter.
    expect(tabDropOutcome({ inside: false, overDropZone: true, canFloat: false, canPopOut: true })).toBe('window');
  });

  it('floats at the docked size within the limits', () => {
    expect(floatSize({ width: 400, height: 300 })).toEqual({ width: 400, height: 300 });
    expect(floatSize({ width: 1600, height: 900 })).toEqual(FLOAT_MAX);
    expect(floatSize({ width: 50, height: 20 })).toEqual(FLOAT_MIN);
    expect(floatSize({ width: 300.4, height: 200.6 })).toEqual({ width: 300, height: 201 });
    expect(floatSize(null)).toEqual(FLOAT_MAX);
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

  it('has a hint only for floating and new windows', () => {
    expect(cueHint('float')).toBe('Release to float');
    expect(cueHint('window')).toBe('Release to open in a new window');
    expect(cueHint('dock')).toBeNull();
    expect(cueHint('none')).toBeNull();
  });
});

describe('FloatCue', () => {
  let cue: FloatCue;
  beforeEach(() => (cue = new FloatCue()));
  afterEach(() => cue.dispose());

  const ghost = () => document.querySelector<HTMLElement>('.imt-float-cue')!;
  const live = () => document.querySelector<HTMLElement>('[aria-live="polite"].sr-only')!;
  const rect = { x: 10, y: 20, width: 300, height: 200 };

  it('starts hidden, with a polite live region and a ghost hidden from assistive technology', () => {
    expect(ghost().hidden).toBe(true);
    expect(ghost().getAttribute('aria-hidden')).toBe('true');
    expect(live().textContent).toBe('');
    expect(cue.shown).toBeNull();
  });

  it('draws the ghost with the hint and announces it once per change', () => {
    cue.show('float', rect);
    expect(ghost().hidden).toBe(false);
    expect(ghost().dataset['outcome']).toBe('float');
    expect(ghost().textContent).toBe('Release to float');
    expect([ghost().style.left, ghost().style.top, ghost().style.width, ghost().style.height]).toEqual(['10px', '20px', '300px', '200px']);
    expect(live().textContent).toBe('Release to float');
    cue.show('float', { ...rect, x: 40 });
    expect(ghost().style.left).toBe('40px');
    cue.show('window', rect);
    expect(ghost().textContent).toBe('Release to open in a new window');
    expect(live().textContent).toBe('Release to open in a new window');
    expect(cue.shown).toBe('window');
  });

  it('clears on a drop zone, on hide and when disposed', () => {
    cue.show('float', rect);
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
    expect(document.querySelector('.imt-float-cue')).toBeNull();
    cue = new FloatCue();
  });
});
