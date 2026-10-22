import { afterNextRender, ChangeDetectionStrategy, Component, computed, DestroyRef, effect, ElementRef, inject, signal, untracked, viewChild } from '@angular/core';
import { NoteItem, NoteRegion } from '../../core/api.types';
import { Platform } from '../../core/platform';
import { imageFilter } from '../../ai/vision/image-files';
import { NoteActions } from '../note-actions';
import { firstLine, severityColor } from '../notes-model';
import { NoteImageStore } from './note-image.store';
import { fitViewport, focusRegion, panBy, Point, regionFromDrag, screenBox, Viewport, zoomAt } from './region-geometry';

type Mode = 'mark' | 'move';

interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

/** A region drawn on screen: whose it is and how it looks. */
interface RegionVm {
  noteId: string | null;
  index: number;
  box: Box;
  color: string;
  label: string;
  focused: boolean;
}

/**
 * The image view (a dock view, ADR 0021 §4): an image of the workspace as the engine decoded it, zoomed and panned,
 * with the regions of its notes drawn over it. "Mark area" draws rectangles (in the original's pixels); "Add note"
 * writes one note on the marked areas. Clicking a region or a note highlights the note's regions.
 */
@Component({
  selector: 'debrief-note-image-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="bar" role="toolbar" aria-label="Image" i18n-aria-label="@@notes.image.toolbar">
      <button type="button" class="btn ghost" [disabled]="store.opening()" (click)="openFile()" data-testid="note-image-open" i18n="@@notes.image.open">Open image…</button>
      @if (store.image(); as img) {
        <span class="name ellipsis" [title]="img.path ?? img.name">{{ img.name }}</span>
        <span class="faint size">{{ img.width }} × {{ img.height }}</span>
        <span class="spacer"></span>
        <div class="seg" role="radiogroup" aria-label="Tool" i18n-aria-label="@@notes.image.tool">
          <button type="button" class="btn ghost small" role="radio" [attr.aria-checked]="mode() === 'mark'" (click)="mode.set('mark')" data-testid="note-image-mark" i18n="@@notes.image.mark">Mark area</button>
          <button type="button" class="btn ghost small" role="radio" [attr.aria-checked]="mode() === 'move'" (click)="mode.set('move')" i18n="@@notes.image.move">Move</button>
        </div>
        <button type="button" class="btn ghost small" (click)="zoom(1 / 1.25)" aria-label="Zoom out" i18n-aria-label="@@notes.image.zoomOut">−</button>
        <span class="faint zoomv" data-testid="note-image-zoom">{{ percent() }} %</span>
        <button type="button" class="btn ghost small" (click)="zoom(1.25)" aria-label="Zoom in" i18n-aria-label="@@notes.image.zoomIn">+</button>
        <button type="button" class="btn ghost small" (click)="fit()" i18n="@@notes.image.fit">Fit</button>
      }
    </div>
    @if (store.error(); as e) {
      <p class="err" role="alert" data-testid="note-image-error">
        {{ e }}
        <button type="button" class="link" (click)="store.retry()" i18n="@@notes.image.retry">Retry</button>
        <button type="button" class="link" (click)="store.dismissError()" i18n="@@notes.image.dismiss">Dismiss</button>
      </p>
    }
    <div class="body">
      <div
        #stage
        class="stage"
        [class.mark]="mode() === 'mark'"
        tabindex="0"
        role="application"
        aria-label="Image; drag to mark an area, + and − zoom, 0 fits"
        i18n-aria-label="@@notes.image.stage"
        data-testid="note-image-stage"
        (pointerdown)="down($event)"
        (pointermove)="move($event)"
        (pointerup)="up($event)"
        (pointercancel)="drag.set(null)"
        (wheel)="wheel($event)"
        (keydown)="key($event)"
      >
        @if (store.image(); as img) {
          <img
            class="img"
            [src]="img.dataUrl"
            [alt]="img.name"
            draggable="false"
            [style.left.px]="vp().panX"
            [style.top.px]="vp().panY"
            [style.width.px]="img.width * vp().scale"
            [style.height.px]="img.height * vp().scale"
            data-testid="note-image-img"
          />
          @for (r of regions(); track r.noteId + ':' + r.index) {
            <button
              type="button"
              class="region"
              [class.focused]="r.focused"
              [style.left.px]="r.box.left"
              [style.top.px]="r.box.top"
              [style.width.px]="r.box.width"
              [style.height.px]="r.box.height"
              [style.--c]="r.color"
              [attr.aria-label]="r.label"
              [title]="r.label"
              (pointerdown)="$event.stopPropagation()"
              (click)="store.focus(r.noteId)"
              data-testid="note-region"
            ></button>
          }
          @for (r of pending(); track $index) {
            <div class="region pending" [style.left.px]="box(r).left" [style.top.px]="box(r).top" [style.width.px]="box(r).width" [style.height.px]="box(r).height" data-testid="note-region-pending"></div>
          }
          @if (dragBox(); as b) {
            <div class="region pending" [style.left.px]="b.left" [style.top.px]="b.top" [style.width.px]="b.width" [style.height.px]="b.height"></div>
          }
        } @else if (!store.opening()) {
          <p class="empty muted" i18n="@@notes.image.empty">Open a screenshot or another image of the workspace, mark the areas that matter and write what you see.</p>
        }
      </div>
      @if (store.image()) {
        <aside class="side" aria-label="Notes on this image" i18n-aria-label="@@notes.image.notes">
          @if (pending().length) {
            <div class="pending-bar">
              <span>{{ pendingLine() }}</span>
              <button type="button" class="btn primary small" (click)="addNote()" data-testid="note-image-add" i18n="@@notes.image.add">Add note…</button>
              <button type="button" class="btn ghost small" (click)="pending.set([])" i18n="@@notes.image.clear">Clear</button>
            </div>
          } @else {
            <p class="faint hint" i18n="@@notes.image.hint">Choose "Mark area" and drag over the image to mark an area.</p>
          }
          <ul class="list">
            @for (n of store.notes(); track n.id) {
              <li class="item" [class.focused]="store.focused() === n.id" [style.--c]="color(n)">
                <button type="button" class="link-btn" (click)="store.focus(n.id)" data-testid="note-image-note">{{ title(n) }}</button>
                <button type="button" class="btn ghost small" (click)="actions.edit(n)" i18n="@@notes.image.edit">Edit</button>
                <button type="button" class="btn ghost small" (click)="actions.remove(n)" i18n="@@notes.image.delete">Delete</button>
              </li>
            }
          </ul>
        </aside>
      }
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; height: 100%; min-height: 0; font-size: var(--fs-md); background: var(--bg-1); }
    .bar { display: flex; align-items: center; gap: 6px; padding: 6px 8px; border-bottom: 1px solid var(--border); flex-wrap: wrap; }
    .name { font-weight: 600; max-width: 30ch; }
    .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .spacer { flex: 1; }
    .size, .zoomv { font-size: var(--fs-xs); }
    .seg { display: inline-flex; gap: 2px; }
    .seg [aria-checked='true'] { background: var(--bg-3); color: var(--text); }
    .small { font-size: var(--fs-xs); }
    .err { color: var(--danger-text); margin: 0; padding: 4px 12px; }
    .link { all: unset; cursor: pointer; color: var(--accent-text); text-decoration: underline; margin-left: 6px; }
    .body { flex: 1; min-height: 0; display: flex; }
    .stage { position: relative; flex: 1; min-width: 0; overflow: hidden; touch-action: none; cursor: grab; background: var(--bg-2); }
    .stage.mark { cursor: crosshair; }
    .stage:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
    .img { position: absolute; max-width: none; user-select: none; pointer-events: none; image-rendering: auto; }
    .region { position: absolute; box-sizing: border-box; border: 2px solid var(--c, var(--accent)); background: color-mix(in srgb, var(--c, var(--accent)) 12%, transparent); padding: 0; cursor: pointer; }
    .region.focused { border-width: 3px; box-shadow: 0 0 0 2px var(--bg-1), 0 0 0 4px var(--c, var(--accent)); background: color-mix(in srgb, var(--c, var(--accent)) 22%, transparent); }
    .region.pending { border-style: dashed; border-color: var(--text); background: transparent; pointer-events: none; }
    .region:focus-visible { outline: 2px solid var(--focus-ring); }
    .empty { margin: 24px; max-width: 48ch; }
    .side { width: 240px; flex: none; border-left: 1px solid var(--border); overflow: auto; padding: 8px; display: flex; flex-direction: column; gap: 8px; }
    .pending-bar { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
    .hint { margin: 0; font-size: var(--fs-xs); }
    .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
    .item { display: flex; align-items: center; gap: 4px; border-left: 3px solid var(--c, var(--accent)); padding-left: 6px; }
    .item.focused { background: var(--bg-3); }
    .link-btn { all: unset; cursor: pointer; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .link-btn:focus-visible { outline: 2px solid var(--focus-ring); }
  `,
})
export class NoteImageView {
  protected readonly store = inject(NoteImageStore);
  protected readonly actions = inject(NoteActions);
  private readonly platform = inject(Platform);
  private readonly stage = viewChild.required<ElementRef<HTMLElement>>('stage');

  protected readonly mode = signal<Mode>('mark');
  protected readonly vp = signal<Viewport>({ scale: 1, panX: 0, panY: 0 });
  private readonly boxSize = signal({ w: 0, h: 0 });
  /** The drag in progress (screen points relative to the stage). */
  protected readonly drag = signal<{ from: Point; to: Point; mode: Mode } | null>(null);
  /** Areas marked for the next note. */
  protected readonly pending = signal<NoteRegion[]>([]);

  protected readonly percent = computed(() => Math.round(this.vp().scale * 100));
  protected readonly regions = computed<RegionVm[]>(() => {
    const vp = this.vp();
    const focused = this.store.focused();
    return this.store.notes().flatMap((n) =>
      n.anchor.kind === 'image'
        ? n.anchor.regions.map((r, index) => ({ noteId: n.id, index, box: screenBox(r, vp), color: severityColor(n.severity), label: this.title(n), focused: n.id === focused }))
        : [],
    );
  });
  protected readonly dragBox = computed<Box | null>(() => {
    const d = this.drag();
    const img = this.store.image();
    if (!d || d.mode !== 'mark' || !img) return null;
    const r = regionFromDrag(d.from, d.to, this.vp(), img.width, img.height);
    return r ? screenBox(r, this.vp()) : null;
  });
  protected readonly pendingLine = computed(() => {
    const n = this.pending().length;
    return n === 1 ? $localize`:@@notes.image.pendingOne:1 area marked` : $localize`:@@notes.image.pendingMany:${n}:count: areas marked`;
  });

  constructor() {
    const destroy = inject(DestroyRef);
    afterNextRender(() => {
      const el = this.stage().nativeElement;
      const measure = () => this.boxSize.set({ w: el.clientWidth, h: el.clientHeight });
      measure();
      if (typeof ResizeObserver !== 'undefined') {
        const ro = new ResizeObserver(measure);
        ro.observe(el);
        destroy.onDestroy(() => ro.disconnect());
      }
    });
    // A new image fits the view; marked areas belong to the image they were drawn on.
    effect(() => {
      const key = this.store.image()?.key;
      this.boxSize();
      untracked(() => {
        if (key !== this.fittedKey) {
          this.fittedKey = key;
          this.pending.set([]);
          this.fit();
        }
      });
    });
    // A highlighted note's first region comes into view.
    effect(() => {
      const id = this.store.focused();
      untracked(() => {
        const n = id ? this.store.notes().find((x) => x.id === id) : undefined;
        const r = n?.anchor.kind === 'image' ? n.anchor.regions[0] : undefined;
        const { w, h } = this.boxSize();
        if (r && w > 0 && h > 0) this.vp.set(focusRegion(r, w, h));
      });
    });
  }
  private fittedKey: string | undefined;

  protected title(n: NoteItem): string {
    return firstLine(n.text, 60);
  }
  protected color(n: NoteItem): string {
    return severityColor(n.severity);
  }
  protected box(r: NoteRegion): Box {
    return screenBox(r, this.vp());
  }

  protected fit(): void {
    const img = this.store.image();
    const { w, h } = this.boxSize();
    if (img && w > 0 && h > 0) this.vp.set(fitViewport(img.width, img.height, w, h));
  }

  protected zoom(factor: number, at?: Point): void {
    const { w, h } = this.boxSize();
    this.vp.update((v) => zoomAt(v, factor, at ?? { x: w / 2, y: h / 2 }));
  }

  private local(e: PointerEvent | WheelEvent): Point {
    const r = this.stage().nativeElement.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  }

  protected down(e: PointerEvent): void {
    if (!this.store.image() || e.button !== 0) return;
    const p = this.local(e);
    (e.target as Element).setPointerCapture?.(e.pointerId);
    this.drag.set({ from: p, to: p, mode: this.mode() });
  }

  protected move(e: PointerEvent): void {
    const d = this.drag();
    if (!d) return;
    const p = this.local(e);
    if (d.mode === 'move') {
      this.vp.update((v) => panBy(v, p.x - d.to.x, p.y - d.to.y));
    }
    this.drag.set({ ...d, to: p });
  }

  protected up(e: PointerEvent): void {
    const d = this.drag();
    this.drag.set(null);
    const img = this.store.image();
    if (!d || !img || d.mode !== 'mark') return;
    const r = regionFromDrag(d.from, this.local(e), this.vp(), img.width, img.height);
    if (r) this.pending.update((list) => [...list, r]);
  }

  protected wheel(e: WheelEvent): void {
    if (!this.store.image()) return;
    e.preventDefault();
    this.zoom(e.deltaY < 0 ? 1.15 : 1 / 1.15, this.local(e));
  }

  protected key(e: KeyboardEvent): void {
    if (e.key === '+' || e.key === '=') this.zoom(1.25);
    else if (e.key === '-') this.zoom(1 / 1.25);
    else if (e.key === '0') this.fit();
    else if (e.key === 'Escape') this.pending.set([]);
    else return;
    e.preventDefault();
  }

  protected async openFile(): Promise<void> {
    const path = await this.platform.pickOpenPath($localize`:@@notes.image.pickTitle:Open an image`, imageFilter());
    if (path) await this.store.showPath(path);
  }

  /** Writes a note on the marked areas. */
  protected async addNote(): Promise<void> {
    const img = this.store.image();
    const regions = this.pending();
    if (!img || !regions.length) return;
    const id = await this.actions.write({ kind: 'image', image: { key: img.key, name: img.name, path: img.path, width: img.width, height: img.height }, regions });
    if (id) {
      this.pending.set([]);
      this.store.focus(id);
    }
  }
}
