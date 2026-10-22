import { createDragGhostBackend, MockDragGhostBackend, mockGhostRequested, NoDragGhost, TauriDragGhostBackend } from './drag-ghost-backend';
import { TestBed } from '@angular/core/testing';

describe('drag ghost backends', () => {
  it('the Tauri backend calls the shell commands with their wire arguments', async () => {
    const calls: [string, unknown][] = [];
    const call = (async (cmd: string, args?: unknown) => {
      calls.push([cmd, args]);
      return cmd === 'drag_ghost_start' ? true : cmd === 'drag_ghost_end' ? { x: 1, y: 2, width: 3, height: 4 } : undefined;
    }) as never;
    const b = new TauriDragGhostBackend(call);
    expect(b.available).toBe(true);
    const request = { title: 'Metrics', width: 560, height: 420, grabX: 120, grabY: 12 };
    expect(await b.start(request)).toBe(true);
    await b.show(true);
    expect(await b.end()).toEqual({ x: 1, y: 2, width: 3, height: 4 });
    expect(calls).toEqual([
      ['drag_ghost_start', { request }],
      ['drag_ghost_show', { visible: true }],
      ['drag_ghost_end', undefined],
    ]);
  });

  it('a browser has no ghost unless the mock is asked for', async () => {
    const none = new NoDragGhost();
    expect(none.available).toBe(false);
    expect(await none.start()).toBe(false);
    await none.show();
    expect(await none.end()).toBeNull();
    expect(mockGhostRequested('?mockGhost=1')).toBe(true);
    expect(mockGhostRequested('?mockGhost=0&x=1')).toBe(false);
    const created = TestBed.runInInjectionContext(() => createDragGhostBackend());
    expect(created.available).toBe(false);
    expect(await created.start({ title: 'x', width: 1, height: 1, grabX: 0, grabY: 0 })).toBe(false);
  });

  it('the mock follows the pointer at the grab point while shown and reports where it was', async () => {
    const m = new MockDragGhostBackend(window);
    const move = (x: number, y: number) => window.dispatchEvent(new PointerEvent('pointermove', { clientX: x - window.screenX, clientY: y - window.screenY }));
    await m.show(true);
    expect(m.state.visible).toBe(false);
    expect(await m.start({ title: 'Metrics', snapshot: 'data:image/png;base64,AA==', width: 560, height: 420, grabX: 120, grabY: 12 })).toBe(true);
    expect(m.state.request).toMatchObject({ title: 'Metrics', hasSnapshot: true });
    expect((window as unknown as { __imtMockGhost: unknown }).__imtMockGhost).toBe(m.state);
    move(500, 300);
    expect(m.state.rect).toBeNull();
    await m.show(true);
    expect(m.state.rect).toEqual({ x: 380, y: 288, width: 560, height: 420 });
    move(700, 310);
    expect(m.state.rect).toEqual({ x: 580, y: 298, width: 560, height: 420 });
    expect(await m.end()).toEqual({ x: 580, y: 298, width: 560, height: 420 });
    expect(m.state).toMatchObject({ starts: 1, ends: 1, visible: false });
    await m.start({ title: 'x', width: 300, height: 200, grabX: 0, grabY: 0 });
    expect(await m.end()).toBeNull();
  });
});
