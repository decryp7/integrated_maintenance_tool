import { Component, ElementRef, signal, viewChild } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it } from 'vitest';
import { PANEL_VISIBLE } from '../core/panel-visibility';
import { FakeDock, manualFrames, provideFakeDock, settle } from '../../testing/helpers';
import { OverlayTracker } from '../ui/overlay-tracker';
import { Placement } from './browser.types';
import { hiddenPlacement, pageViewport, placementFor, samePlacement } from './page-viewport';

const rect = { x: 10.004, y: 20, width: 300, height: 200 };
const input = { panelVisible: true, pageOpen: true, documentHidden: false, covered: false, rect, scale: 1.25 };

describe('placementFor', () => {
  it('shows the page only while the panel is visible, a page is open, nothing covers it and the area has a size', () => {
    expect(placementFor(input)).toEqual({ visible: true, rect: { x: 10, y: 20, width: 300, height: 200 }, scale: 1.25 });
    for (const off of [{ panelVisible: false }, { pageOpen: false }, { documentHidden: true }, { covered: true }]) {
      const p = placementFor({ ...input, ...off });
      expect(p.visible, JSON.stringify(off)).toBe(false);
      expect(p.rect, 'the place is still reported').not.toBeNull();
    }
    expect(placementFor({ ...input, rect: { ...rect, width: 0 } })).toEqual({ visible: false, rect: null, scale: 1.25 });
    expect(placementFor({ ...input, rect: null }).visible).toBe(false);
  });

  it('compares placements by value', () => {
    expect(samePlacement(placementFor(input), placementFor({ ...input, rect: { ...rect, x: 10.001 } }))).toBe(true);
    expect(samePlacement(placementFor(input), placementFor({ ...input, covered: true }))).toBe(false);
    expect(samePlacement(null, hiddenPlacement(1))).toBe(false);
    expect(hiddenPlacement(2)).toEqual({ visible: false, rect: null, scale: 2 });
  });
});

@Component({
  template: `@if (shown()) {<div #area class="area"></div>}`,
})
class Host {
  readonly shown = signal(true);
  readonly open = signal(true);
  readonly placed: Placement[] = [];
  readonly area = viewChild<ElementRef<HTMLElement>>('area');
  readonly viewport = pageViewport({ area: this.area, open: this.open, place: (p) => (this.placed.push(p), Promise.resolve(true)) });
}

describe('pageViewport', () => {
  afterEach(() => {
    document.body.replaceChildren();
    TestBed.resetTestingModule();
  });

  function setup() {
    const visible = signal(true);
    const dock = new FakeDock();
    TestBed.configureTestingModule({ providers: [...provideFakeDock(dock), { provide: PANEL_VISIBLE, useValue: visible }] });
    const frames = manualFrames();
    const f = TestBed.createComponent(Host);
    f.detectChanges();
    const area = (f.nativeElement as HTMLElement).querySelector<HTMLElement>('.area')!;
    let box = { left: 100, top: 50, width: 400, height: 300 };
    area.getBoundingClientRect = () => ({ ...box, x: box.left, y: box.top, right: box.left + box.width, bottom: box.top + box.height, toJSON: () => box }) as DOMRect;
    const step = async () => {
      await settle();
      f.detectChanges();
      frames.flush();
      await settle();
    };
    return { f, visible, dock, frames, area, step, setBox: (b: typeof box) => (box = b) };
  }

  it('reports the page area once per change and hides the page with its tab, an overlay or the panel', async () => {
    const { f, visible, dock, frames, area, step, setBox } = setup();
    try {
      const host = f.componentInstance;
      await step();
      expect(host.placed.at(-1)).toEqual({ visible: true, rect: { x: 100, y: 50, width: 400, height: 300 }, scale: window.devicePixelRatio || 1 });
      expect(host.viewport.shown()).toBe(true);
      const sent = host.placed.length;
      host.viewport.schedule();
      await step();
      expect(host.placed.length, 'nothing changed: nothing sent').toBe(sent);
      // The dock moved the panel.
      setBox({ left: 0, top: 50, width: 400, height: 300 });
      dock.layoutChanged.update((n) => n + 1);
      await step();
      expect(host.placed.at(-1)?.rect?.x).toBe(0);
      // A background tab.
      visible.set(false);
      await step();
      expect(host.placed.at(-1)?.visible).toBe(false);
      visible.set(true);
      await step();
      expect(host.placed.at(-1)?.visible).toBe(true);
      // A menu opens over the page: it hides; it closes: the page is back.
      const menu = document.createElement('div');
      menu.setAttribute('data-imt-overlay', '');
      menu.getBoundingClientRect = () => ({ left: 300, top: 60, right: 360, bottom: 120, width: 60, height: 60, x: 300, y: 60, toJSON: () => null }) as DOMRect;
      document.body.appendChild(menu);
      await step();
      await step();
      expect(host.placed.at(-1)?.visible).toBe(false);
      expect(host.viewport.covered()).toBe(true);
      menu.remove();
      await step();
      await step();
      expect(host.placed.at(-1)?.visible).toBe(true);
      // A hold (a drag) hides it too.
      const release = TestBed.inject(OverlayTracker).hold();
      await step();
      expect(host.placed.at(-1)?.visible).toBe(false);
      release();
      await step();
      // No page: hidden.
      host.open.set(false);
      await step();
      expect(host.placed.at(-1)?.visible).toBe(false);
      host.open.set(true);
      await step();
      // The area leaves the document (the panel's content changed): hidden, nothing to place.
      host.shown.set(false);
      await step();
      expect(host.placed.at(-1)).toEqual({ visible: false, rect: null, scale: window.devicePixelRatio || 1 });
      host.shown.set(true);
      await step();
      const again = (f.nativeElement as HTMLElement).querySelector<HTMLElement>('.area')!;
      again.getBoundingClientRect = area.getBoundingClientRect;
      host.viewport.schedule();
      await step();
      expect(host.placed.at(-1)?.visible, 'the new area is placed').toBe(true);
      // The panel goes: the page hides.
      const before = host.placed.length;
      f.destroy();
      expect(host.placed.length).toBe(before + 1);
      expect(host.placed.at(-1)).toEqual(hiddenPlacement(window.devicePixelRatio || 1));
    } finally {
      frames.restore();
    }
  });

  it('follows the window: a resize and a hidden document', async () => {
    const { f, frames, step, setBox } = setup();
    try {
      const host = f.componentInstance;
      await step();
      setBox({ left: 100, top: 50, width: 200, height: 300 });
      window.dispatchEvent(new Event('resize'));
      await step();
      expect(host.placed.at(-1)?.rect?.width).toBe(200);
      Object.defineProperty(document, 'hidden', { configurable: true, get: () => true });
      document.dispatchEvent(new Event('visibilitychange'));
      await step();
      expect(host.placed.at(-1)?.visible).toBe(false);
      Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
      document.dispatchEvent(new Event('visibilitychange'));
      await step();
      expect(host.placed.at(-1)?.visible).toBe(true);
    } finally {
      frames.restore();
    }
  });
});
