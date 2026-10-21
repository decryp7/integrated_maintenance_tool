import { TestBed } from '@angular/core/testing';
import { all, el, provideTinyBackend, settle, text } from '../../../testing/helpers';
import { at } from '../../../testing/must';
import { Platform } from '../../core/platform';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { DialogService } from '../../ui/dialog.service';
import { MockNotesBackend } from '../mock/mock-notes-backend';
import { NotesBackend } from '../notes-backend';
import { NotesStore } from '../notes.store';
import { NoteImageStore } from './note-image.store';
import { NoteImageView } from './note-image-view';
import { fitViewport, focusRegion, MAX_SCALE, MIN_SCALE, panBy, regionAt, regionFromDrag, screenBox, toImage, toScreen, zoomAt } from './region-geometry';

describe('region geometry', () => {
  it('fits an image into a box, never enlarging it', () => {
    expect(fitViewport(800, 400, 400, 400)).toEqual({ scale: 0.5, panX: 0, panY: 100 });
    expect(fitViewport(100, 50, 400, 400)).toEqual({ scale: 1, panX: 150, panY: 175 });
    expect(fitViewport(0, 50, 400, 400)).toEqual({ scale: 1, panX: 0, panY: 0 });
  });

  it('converts between screen and image pixels and zooms around a point', () => {
    const vp = { scale: 2, panX: 10, panY: 20 };
    expect(toImage({ x: 30, y: 40 }, vp)).toEqual({ x: 10, y: 10 });
    expect(toScreen({ x: 10, y: 10 }, vp)).toEqual({ x: 30, y: 40 });
    const z = zoomAt(vp, 2, { x: 30, y: 40 });
    expect(z.scale).toBe(4);
    expect(toImage({ x: 30, y: 40 }, z)).toEqual({ x: 10, y: 10 });
    expect(zoomAt(vp, 1000, { x: 0, y: 0 }).scale).toBe(MAX_SCALE);
    expect(zoomAt(vp, 0.0001, { x: 0, y: 0 }).scale).toBe(MIN_SCALE);
    expect(panBy(vp, 5, -5)).toEqual({ scale: 2, panX: 15, panY: 15 });
  });

  it('turns a drag in any direction into a region in whole image pixels, clamped to the image', () => {
    const vp = { scale: 0.5, panX: 0, panY: 0 };
    expect(regionFromDrag({ x: 10, y: 10 }, { x: 60, y: 35 }, vp, 800, 400)).toEqual({ x: 20, y: 20, w: 100, h: 50 });
    expect(regionFromDrag({ x: 60, y: 35 }, { x: 10, y: 10 }, vp, 800, 400)).toEqual({ x: 20, y: 20, w: 100, h: 50 });
    expect(regionFromDrag({ x: -50, y: -50 }, { x: 1000, y: 1000 }, vp, 800, 400)).toEqual({ x: 0, y: 0, w: 800, h: 400 });
    expect(regionFromDrag({ x: 10.2, y: 10.2 }, { x: 20.7, y: 20.7 }, { scale: 1, panX: 0, panY: 0 }, 100, 100)).toEqual({ x: 10, y: 10, w: 11, h: 11 });
    expect(regionFromDrag({ x: 10, y: 10 }, { x: 10.4, y: 30 }, vp, 800, 400)).toBeNull();
    expect(regionFromDrag({ x: 500, y: 500 }, { x: 600, y: 600 }, vp, 800, 400)).toBeNull();
  });

  it('places regions on screen, hit-tests the innermost and centres one', () => {
    const vp = { scale: 2, panX: 5, panY: 5 };
    expect(screenBox({ x: 10, y: 20, w: 5, h: 5 }, vp)).toEqual({ left: 25, top: 45, width: 10, height: 10 });
    const regions = [
      { x: 0, y: 0, w: 100, h: 100 },
      { x: 10, y: 10, w: 10, h: 10 },
    ];
    expect(regionAt({ x: 15, y: 15 }, regions)).toBe(1);
    expect(regionAt({ x: 50, y: 50 }, regions)).toBe(0);
    expect(regionAt({ x: 150, y: 50 }, regions)).toBe(-1);
    const f = focusRegion({ x: 100, y: 100, w: 50, h: 25 }, 400, 200);
    expect(toScreen({ x: 125, y: 112.5 }, f)).toEqual({ x: 200, y: 100 });
    expect(f.scale).toBe(4);
  });
});

describe('NoteImageView', () => {
  async function setup() {
    const backend = new MockNotesBackend();
    const context = { showView: vi.fn() };
    TestBed.configureTestingModule({
      providers: [...provideTinyBackend(), { provide: NotesBackend, useValue: backend }, { provide: COMMAND_CONTEXT, useValue: context }],
    });
    const platform = { pickOpenPath: vi.spyOn(TestBed.inject(Platform), 'pickOpenPath').mockResolvedValue('/shots/screen.png') };
    const f = TestBed.createComponent(NoteImageView);
    await settle(f);
    const stage = el<HTMLElement>(f, '[data-testid=note-image-stage]');
    // The test DOM has no layout: give the stage a size and place it at the origin.
    Object.defineProperty(stage, 'clientWidth', { value: 400 });
    Object.defineProperty(stage, 'clientHeight', { value: 225 });
    stage.getBoundingClientRect = () => ({ left: 0, top: 0, right: 400, bottom: 225, width: 400, height: 225, x: 0, y: 0, toJSON: () => ({}) });
    return { f, backend, stage, platform };
  }
  const pointer = (type: string, x: number, y: number) => new PointerEvent(type, { clientX: x, clientY: y, button: 0, bubbles: true });

  it('opens an image, marks areas by dragging and writes one note on them', async () => {
    const { f, stage, platform } = await setup();
    expect(text(f)).toContain('Open a screenshot');
    el<HTMLButtonElement>(f, '[data-testid=note-image-open]').click();
    await settle(f);
    expect(platform.pickOpenPath).toHaveBeenCalled();
    // Fit the 800×450 image into 400×225 (the size observed when the view was created).
    window.dispatchEvent(new Event('resize'));
    (f.componentInstance as unknown as { boxSize: { set(v: unknown): void } }).boxSize.set({ w: 400, h: 225 });
    (f.componentInstance as unknown as { fit(): void }).fit();
    await settle(f);
    expect(el(f, '[data-testid=note-image-zoom]').textContent).toContain('50');
    stage.dispatchEvent(pointer('pointerdown', 10, 10));
    stage.dispatchEvent(pointer('pointermove', 60, 35));
    stage.dispatchEvent(pointer('pointerup', 60, 35));
    stage.dispatchEvent(pointer('pointerdown', 100, 100));
    stage.dispatchEvent(pointer('pointerup', 100, 100)); // a click marks nothing
    await settle(f);
    expect(all(f, '[data-testid=note-region-pending]')).toHaveLength(1);
    const dialogs = TestBed.inject(DialogService);
    el<HTMLButtonElement>(f, '[data-testid=note-image-add]').click();
    const dlg = dialogs.stack().at(-1);
    expect(dlg?.options.inputs?.['anchor']).toMatchObject({ kind: 'image', image: { name: 'screen.png', width: 800, height: 450 }, regions: [{ x: 20, y: 20, w: 100, h: 50 }] });
    const n = await TestBed.inject(NotesStore).add({ anchor: dlg?.options.inputs?.['anchor'] as never, text: 'red banner' });
    dlg?.ref.close(n!.id);
    await settle(f);
    expect(all(f, '[data-testid=note-region-pending]')).toHaveLength(0);
    const region = at(all<HTMLButtonElement>(f, '[data-testid=note-region]'), 0);
    expect(region.getAttribute('aria-label')).toBe('red banner');
    expect(region.classList).toContain('focused');
    expect(at(all(f, '[data-testid=note-image-note]'), 0).textContent).toContain('red banner');
  });

  it('pans in Move mode, zooms with the keyboard and wheel, and focuses a note from its region', async () => {
    const { f, stage } = await setup();
    await TestBed.inject(NoteImageStore).showPath('/shots/a.png');
    await settle(f);
    const store = TestBed.inject(NotesStore);
    const img = TestBed.inject(NoteImageStore).image()!;
    await store.add({ anchor: { kind: 'image', image: { ...img }, regions: [{ x: 0, y: 0, w: 10, h: 10 }] }, text: 'one' });
    await settle(f);
    const zoom = () => el(f, '[data-testid=note-image-zoom]').textContent?.trim();
    const before = zoom();
    stage.dispatchEvent(new KeyboardEvent('keydown', { key: '+' }));
    await settle(f);
    expect(zoom()).not.toBe(before);
    stage.dispatchEvent(new KeyboardEvent('keydown', { key: '-' }));
    stage.dispatchEvent(new WheelEvent('wheel', { deltaY: -1, clientX: 5, clientY: 5, cancelable: true }));
    await settle(f);
    // Move mode pans instead of marking.
    at(all<HTMLButtonElement>(f, '[role=radio]'), 1).click();
    await settle(f);
    stage.dispatchEvent(pointer('pointerdown', 10, 10));
    stage.dispatchEvent(pointer('pointermove', 40, 10));
    stage.dispatchEvent(pointer('pointerup', 40, 10));
    await settle(f);
    expect(all(f, '[data-testid=note-region-pending]')).toHaveLength(0);
    at(all<HTMLButtonElement>(f, '[data-testid=note-region]'), 0).click();
    await settle(f);
    expect(TestBed.inject(NoteImageStore).focused()).toBe('n1');
    stage.dispatchEvent(new KeyboardEvent('keydown', { key: '0' }));
    stage.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    stage.dispatchEvent(new KeyboardEvent('keydown', { key: 'a' }));
  });

  it('shows a failed open with Retry', async () => {
    const { f, backend } = await setup();
    vi.spyOn(backend, 'openNoteImage').mockRejectedValueOnce(new Error('the image is 40.0 MB'));
    const images = TestBed.inject(NoteImageStore);
    await images.showPath('/big.png');
    await settle(f);
    expect(el(f, '[data-testid=note-image-error]').textContent).toContain('40.0 MB');
    expect(await images.retry()).toBe(true);
    await settle(f);
    expect(images.image()?.name).toBe('big.png');
    expect(await images.showKey(images.image()!.key, 'n9')).toBe(true);
    expect(images.focused()).toBe('n9');
    images.dismissError();
    images.close();
    expect(images.image()).toBeNull();
  });
});
