import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it } from 'vitest';
import { manualFrames, settle } from '../../testing/helpers';
import { covered, intersects, OverlayTracker } from './overlay-tracker';

const box = (left: number, top: number, right: number, bottom: number) => ({ left, top, right, bottom });

/** An element with a fixed box (jsdom lays nothing out). */
function placed(tag: string, attrs: Record<string, string>, b: ReturnType<typeof box> | null): HTMLElement {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  const r = b ?? box(0, 0, 0, 0);
  el.getBoundingClientRect = () => ({ ...r, x: r.left, y: r.top, width: r.right - r.left, height: r.bottom - r.top, toJSON: () => r }) as DOMRect;
  document.body.appendChild(el);
  return el;
}

describe('overlay geometry', () => {
  it('counts shared areas, not touching edges or empty boxes', () => {
    const page = box(100, 100, 400, 300);
    expect(intersects(page, box(390, 290, 500, 500))).toBe(true);
    expect(intersects(page, box(400, 100, 500, 300))).toBe(false);
    expect(intersects(page, box(0, 0, 100, 100))).toBe(false);
    expect(covered(page, [box(0, 0, 50, 50), box(150, 150, 160, 160)])).toBe(true);
    expect(covered(page, [box(150, 150, 150, 160)])).toBe(false);
    expect(covered(page, [])).toBe(false);
  });
});

describe('OverlayTracker', () => {
  afterEach(() => {
    document.body.replaceChildren();
    TestBed.resetTestingModule();
  });

  it('finds marked overlays and CDK panes over an area, and holds hide it anywhere', () => {
    const t = TestBed.inject(OverlayTracker);
    const page = box(100, 100, 400, 300);
    expect(t.covers(page)).toBe(false);
    placed('div', { class: 'not-an-overlay' }, box(0, 0, 1000, 1000));
    expect(t.covers(page)).toBe(false);
    const menu = placed('div', { 'data-imt-overlay': '' }, box(380, 280, 480, 380));
    expect(t.covers(page)).toBe(true);
    menu.remove();
    placed('div', { class: 'cdk-overlay-pane' }, box(0, 0, 50, 50));
    expect(t.covers(page), 'elsewhere').toBe(false);
    placed('div', { 'data-imt-overlay': '' }, null);
    expect(t.covers(page), 'hidden (no area)').toBe(false);
    expect(t.boxes()).toHaveLength(1);
    const release = t.hold();
    expect(t.held()).toBe(true);
    expect(t.covers(box(0, 0, 1, 1))).toBe(true);
    release();
    release();
    expect(t.held()).toBe(false);
  });

  it('ticks once per frame while watched, and stops observing when nobody watches', async () => {
    const frames = manualFrames();
    try {
      const t = TestBed.inject(OverlayTracker);
      let destroy = () => undefined as void;
      t.watch({ onDestroy: (fn: () => void) => ((destroy = fn), () => undefined) });
      const before = t.changes();
      placed('div', { 'data-imt-overlay': '' }, box(0, 0, 10, 10));
      placed('div', {}, null);
      await settle();
      expect(frames.pending).toBe(1);
      frames.flush();
      expect(t.changes()).toBe(before + 1);
      document.body.firstElementChild?.removeAttribute('data-imt-overlay');
      await settle();
      frames.flush();
      expect(t.changes(), 'an attribute change counts too').toBe(before + 2);
      destroy();
      placed('div', { 'data-imt-overlay': '' }, null);
      await settle();
      expect(frames.pending).toBe(0);
    } finally {
      frames.restore();
    }
  });
});
