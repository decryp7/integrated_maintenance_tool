import { afterNextRender, ChangeDetectionStrategy, Component, computed, DestroyRef, effect, ElementRef, inject, input, linkedSignal, output, signal, untracked, viewChild } from '@angular/core';
import { FlameGraph, FlameRect, FrameDto, MetricDto } from '../../core/api.types';
import { formatMetric, formatPct, formatPp, measurementLabel } from '../profiling-format';
import { buildFlameModel, chainTo, FlameModel, FlameOrientation, flameKey, hitTest, topRects } from './flame-model';
import { ColorBy, diffColor, FALLBACK_PALETTE, FlamePalette, frameColor, KIND_LABEL, KIND_SLOT, maxAbsDelta, modulesByValue, resolvePalette, stickySlots } from './flame-colors';
import { depthAt, drawFlame, FLAME_ROW_H, FlameCtx, rowY } from './flame-draw';
import { MeasurementTag, nextTagId } from '../profiling-shared';

/** How many frames the accessible list shows. */
const LIST_LIMIT = 100;

interface Tip {
  x: number;
  y: number;
  index: number;
}

/**
 * Draws the engine's pre-laid-out flame rectangles on a canvas (fast for thousands of frames) and
 * keeps them accessible: the chart is focusable and navigable with the arrow keys (announced in a
 * live region), and a list of the largest visible frames offers the same zoom actions as buttons.
 */
@Component({
  selector: 'imt-flame-chart',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [MeasurementTag],
  template: `
    <div class="legend" role="group" aria-label="Colour legend" i18n-aria-label="@@profiling.flame.legendLabel">
      @if (diff()) {
        <span class="lg" i18n="@@profiling.flame.legend.less"><span class="sw" [style.background]="palette().less"></span>− less than the baseline ({{ pp(-scale()) // i18n(ph: "delta") }})</span>
        <span class="ramp" aria-hidden="true" [style.background]="'linear-gradient(90deg,' + palette().less + ',' + palette().neutral + ',' + palette().more + ')'"></span>
        <span class="lg" i18n="@@profiling.flame.legend.more"><span class="sw" [style.background]="palette().more"></span>+ more than the baseline ({{ pp(scale()) // i18n(ph: "delta") }})</span>
        <span class="lg" i18n="@@profiling.flame.legend.same"><span class="sw" [style.background]="palette().neutral"></span>no change</span>
      } @else {
        @for (l of legend(); track l.label) {
          <span class="lg"><span class="sw" [style.background]="l.color"></span>{{ l.label }}</span>
        }
      }
      @if (graph(); as g) {
        <span class="spacer"></span>
        <imt-measurement [m]="g.measurement" />
      }
    </div>
    <div class="scroll" #scroll>
      <div
        class="surface"
        #surface
        tabindex="0"
        role="group"
        aria-roledescription="flame graph"
        i18n-aria-roledescription="@@profiling.flame.roleDescription"
        [attr.aria-label]="ariaLabel()"
        [attr.aria-describedby]="helpId"
        [style.height.px]="height()"
        (keydown)="onKey($event)"
        (pointermove)="onMove($event)"
        (pointerleave)="tip.set(null)"
        (click)="onClick($event)"
      >
        <canvas #canvas aria-hidden="true"></canvas>
      </div>
    </div>
    <span [id]="helpId" class="sr">{{ helpText() }}</span>
    <div class="sr" aria-live="polite" data-testid="flame-announce">{{ announce() }}</div>
    @if (tipRect(); as t) {
      <div class="popover tip" role="tooltip" [style.left.px]="t.x + 14" [style.top.px]="t.y + 12">
        <strong class="tname">{{ frameName(t.rect) }}</strong>
        @if (frameOf(t.rect); as f) { <span class="faint">{{ f.module ?? noModule }} · {{ kindLabel(f) }}</span> }
        <span class="mono">{{ totalLine(t.rect) }}</span>
        <span class="mono" i18n="@@profiling.flame.tip.self">Self {{ value(t.rect.selfValue) // i18n(ph: "value") }}</span>
        @if (t.rect.delta !== undefined) {
          <span class="mono" i18n="@@profiling.flame.tip.baseline">Baseline {{ value(t.rect.baselineValue ?? 0) // i18n(ph: "baseline") }} · change {{ pp(t.rect.delta * 100) // i18n(ph: "delta") }}</span>
        }
        <span class="faint" i18n="@@profiling.flame.tip.hint">{{ measurement() // i18n(ph: "measurement") }} · click to zoom</span>
      </div>
    }
    <details class="frames">
      <summary>{{ framesSummary() }}</summary>
      <table class="grid">
        <thead><tr><th scope="col" i18n="@@profiling.flame.col.frame">Frame</th><th scope="col" i18n="@@profiling.flame.col.depth">Depth</th><th scope="col" class="num" i18n="@@profiling.flame.col.total">Total</th><th scope="col" class="num" i18n="@@profiling.flame.col.share">Share</th>@if (diff()) { <th scope="col" class="num" i18n="@@profiling.flame.col.change">Change</th> }</tr></thead>
        <tbody>
          @for (i of listed(); track i) {
            @if (model().rects[i]; as r) {
              <tr [class.sel]="current() === i">
                <td><button class="link" type="button" (click)="zoomAt(i)" [attr.aria-label]="zoomLabel(r)">{{ frameName(r) }}</button>@if (r.matched) { <span class="tag" i18n="@@profiling.flame.match">match</span> }</td>
                <td>{{ r.depth }}</td>
                <td class="num">{{ value(r.value) }}</td>
                <td class="num">{{ pct(r.value) }}</td>
                @if (diff()) { <td class="num">{{ pp((r.delta ?? 0) * 100) }}</td> }
              </tr>
            }
          }
        </tbody>
      </table>
    </details>
  `,
  styles: `
    :host { display: flex; flex-direction: column; min-height: 0; flex: 1; position: relative; }
    .legend { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; padding: 4px 12px; font-size: var(--fs-xs); color: var(--muted); }
    .lg { display: inline-flex; align-items: center; gap: 5px; white-space: nowrap; }
    .sw { width: 12px; height: 10px; border-radius: 2px; display: inline-block; }
    .ramp { width: 90px; height: 8px; border-radius: 4px; }
    .spacer { flex: 1; }
    .scroll { flex: 1; min-height: 80px; overflow: auto; padding: 0 12px; }
    .surface { position: relative; width: 100%; cursor: pointer; }
    .surface:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
    canvas { position: absolute; left: 0; top: 0; }
    .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
    .tip { position: fixed; display: flex; flex-direction: column; gap: 2px; max-width: 420px; pointer-events: none; z-index: 10; font-size: var(--fs-sm); }
    .tname { word-break: break-all; color: var(--text); }
    .faint { color: var(--faint); }
    .mono { font-family: var(--font-mono); }
    .frames { flex: none; max-height: 180px; overflow: auto; padding: 2px 12px 6px; font-size: var(--fs-sm); }
    .frames summary { cursor: pointer; color: var(--muted); padding: 2px 0; }
    table.grid { border-collapse: collapse; width: 100%; }
    table.grid th, table.grid td { padding: 2px 8px; border-bottom: 1px solid var(--border); text-align: left; white-space: nowrap; }
    table.grid .num { text-align: right; font-family: var(--font-mono); }
    tr.sel td { background: var(--bg-sel); }
    .link { background: none; border: 0; padding: 0; font: inherit; color: var(--chip-text); cursor: pointer; text-align: left; }
    .link:focus-visible { outline: 2px solid var(--focus-ring); }
    .tag { margin-left: 6px; font-family: var(--font-mono); font-size: var(--fs-2xs); padding: 0 4px; border: 1px solid var(--border-3); border-radius: 3px; }
  `,
})
export class FlameChart {
  readonly graph = input<FlameGraph | null>(null);
  readonly orientation = input<FlameOrientation>('flame');
  readonly colorBy = input<ColorBy>('kind');
  readonly diff = input(false);
  readonly metrics = input<MetricDto[]>([]);
  readonly label = input($localize`:@@profiling.flame.defaultLabel:Flame graph`);
  /** Zoom request: the frame ids from depth 0 down to the chosen rectangle. */
  readonly zoom = output<number[]>();
  readonly zoomOut = output<void>();
  /** The keyboard/selection cursor moved to a frame. */
  readonly selected = output<FrameDto>();

  private readonly canvas = viewChild<ElementRef<HTMLCanvasElement>>('canvas');
  private readonly surface = viewChild<ElementRef<HTMLElement>>('surface');
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);
  protected readonly width = signal(800);
  /** The keyboard cursor; a new graph or orientation resets it. */
  protected readonly current = linkedSignal<number | null>(() => {
    this.graph();
    this.orientation();
    return null;
  });
  protected readonly tip = signal<Tip | null>(null);
  protected readonly palette = signal<FlamePalette>(FALLBACK_PALETTE);
  protected readonly listLimit = LIST_LIMIT;
  protected readonly noModule = $localize`:@@profiling.flame.noModule:no module`;
  protected readonly helpId = nextTagId('flame-help');
  private moduleSlots = new Map<string, number>();

  protected readonly model = computed<FlameModel>(() => buildFlameModel(this.graph() ?? { rects: [], frames: [], focusFrames: [], focusValue: 0, grandTotal: 0, matchedValue: 0, matchedPct: 0, truncated: false, maxDepth: 0, metric: 'samples', measurement: 'sampled' }));
  protected readonly height = computed(() => (this.model().maxDepth + 1) * FLAME_ROW_H);
  protected readonly scale = computed(() => maxAbsDelta(this.model()) * 100);
  protected readonly listed = computed(() => topRects(this.model(), LIST_LIMIT));
  private readonly slots = computed(() => {
    this.moduleSlots = stickySlots(this.moduleSlots, modulesByValue(this.model()));
    return this.moduleSlots;
  });
  protected readonly legend = computed(() => {
    const p = this.palette();
    if (this.colorBy() === 'kind') {
      return (['managed', 'native', 'kernel', 'unknown'] as const).map((k) => ({ label: KIND_LABEL[k], color: KIND_SLOT[k] === null ? p.other : p.slots[KIND_SLOT[k]!] }));
    }
    const entries = [...this.slots().entries()].filter(([m]) => modulesByValue(this.model()).includes(m)).sort((a, b) => a[1] - b[1]);
    return [...entries.map(([m, slot]) => ({ label: m, color: p.slots[slot] })), { label: $localize`:@@profiling.flame.otherModules:Other modules`, color: p.other }];
  });
  protected readonly summary = computed(() => {
    const g = this.graph();
    if (!g) return $localize`:@@profiling.flame.noData:No data.`;
    const frames = g.rects.length;
    const levels = this.model().maxDepth + 1;
    const parts = [$localize`:@@profiling.flame.summary:${frames}:frames: frames, ${levels}:levels: levels`];
    if (g.truncated) parts.push($localize`:@@profiling.flame.truncated:truncated: more frames than the limit, zoom in for detail`);
    return parts.join('; ');
  });
  protected readonly ariaLabel = computed(() => {
    const label = this.label();
    const summary = this.summary();
    return $localize`:@@profiling.flame.ariaLabel:${label}:label:. ${summary}:summary:`;
  });
  protected readonly helpText = computed(() =>
    this.orientation() === 'flame'
      ? $localize`:@@profiling.flame.help.flame:Arrow keys move between frames (Up to a callee, Down to the caller, Left and Right to neighbours); Enter zooms in, Escape zooms out, Home goes to the top frame.`
      : $localize`:@@profiling.flame.help.icicle:Arrow keys move between frames (Down to a callee, Up to the caller, Left and Right to neighbours); Enter zooms in, Escape zooms out, Home goes to the top frame.`,
  );
  protected readonly framesSummary = computed(() => {
    const count = this.model().rects.length;
    const limit = this.listLimit;
    return count > limit
      ? $localize`:@@profiling.flame.framesLimited:Frames in view (${count}:count:, largest ${limit}:limit: listed)`
      : $localize`:@@profiling.flame.frames:Frames in view (${count}:count:)`;
  });
  protected readonly announce = computed(() => {
    const i = this.current();
    const r = i === null ? null : this.model().rects[i];
    if (!r) return '';
    const change = r.delta !== undefined ? formatPp(r.delta * 100) : null;
    const delta = change !== null ? $localize`:@@profiling.flame.announce.change:, change ${change}:change:` : '';
    const matched = r.matched ? $localize`:@@profiling.flame.announce.matched:, matches the search` : '';
    const name = this.frameName(r);
    const depth = r.depth;
    const total = this.value(r.value);
    const share = this.pct(r.value);
    const self = this.value(r.selfValue);
    return $localize`:@@profiling.flame.announce:${name}:name:, depth ${depth}:depth:, total ${total}:total:, ${share}:share:, self ${self}:self:${delta}:delta:${matched}:matched:`;
  });
  protected readonly tipRect = computed(() => {
    const t = this.tip();
    const rect = t ? this.model().rects[t.index] : null;
    return t && rect ? { x: t.x, y: t.y, rect } : null;
  });

  constructor() {
    const destroy = inject(DestroyRef);
    afterNextRender(() => {
      this.readPalette();
      const el = this.surface()?.nativeElement;
      if (!el) return;
      const ro = new ResizeObserver(() => this.width.set(Math.max(100, el.clientWidth || 800)));
      ro.observe(el);
      destroy.onDestroy(() => ro.disconnect());
    });
    // A flame graph (root at the bottom) scrolls to its root on a new graph.
    effect(() => {
      this.graph();
      const flame = this.orientation() === 'flame';
      untracked(() => {
        const scroller = this.surface()?.nativeElement.parentElement;
        if (scroller) queueMicrotask(() => (scroller.scrollTop = flame ? scroller.scrollHeight : 0));
      });
    });
    effect(() => this.draw());
  }

  /** The surface width in px; the parent asks the engine for a layout at this width. */
  readonly widthPx = computed(() => Math.round(this.width()));

  private readPalette(): void {
    const style = getComputedStyle(this.host.nativeElement);
    this.palette.set(resolvePalette((t) => style.getPropertyValue(t)));
  }

  private colorOf(i: number): string {
    const r = this.model().rects[i];
    if (!r) return this.palette().other;
    if (this.diff()) return diffColor(r.delta, this.scale() / 100, this.palette());
    return frameColor(this.model().frames.get(r.frame), this.colorBy(), this.slots(), this.palette());
  }

  private draw(): void {
    const model = this.model();
    const width = this.width();
    const height = this.height();
    const current = this.current();
    const hovered = this.tip()?.index ?? null;
    const palette = this.palette();
    const search = !!this.graph() && this.graph()!.rects.some((r) => r.matched);
    const canvas = this.canvas()?.nativeElement;
    if (!canvas) return;
    const dpr = typeof window !== 'undefined' ? window.devicePixelRatio || 1 : 1;
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
    canvas.style.width = `${width}px`;
    canvas.style.height = `${height}px`;
    const ctx = canvas.getContext('2d') as (FlameCtx & { setTransform?: (a: number, b: number, c: number, d: number, e: number, f: number) => void }) | null;
    if (!ctx) return;
    ctx.setTransform?.(dpr, 0, 0, dpr, 0, 0);
    untracked(() =>
      drawFlame(ctx, model, { width, height, orientation: this.orientation(), color: (i) => this.colorOf(i), palette, dimUnmatched: search, current, hovered, font: '11px "JetBrains Mono", monospace' }),
    );
  }

  protected frameOf(r: FlameRect): FrameDto | undefined {
    return this.model().frames.get(r.frame);
  }

  protected frameName(r: FlameRect): string {
    const number = r.frame;
    return this.frameOf(r)?.name ?? $localize`:@@profiling.flame.frameNumber:frame #${number}:id:`;
  }

  protected zoomLabel(r: FlameRect): string {
    const name = this.frameName(r);
    return $localize`:@@profiling.flame.zoomTo:Zoom to ${name}:name:`;
  }

  protected totalLine(r: FlameRect): string {
    const value = this.value(r.value);
    const share = this.pct(r.value);
    return this.graph()?.focusValue === this.graph()?.grandTotal
      ? $localize`:@@profiling.flame.tip.totalAll:Total ${value}:value: (${share}:share: of all)`
      : $localize`:@@profiling.flame.tip.totalZoomed:Total ${value}:value: (${share}:share: of the zoomed frame)`;
  }

  protected kindLabel(f: FrameDto): string {
    return KIND_LABEL[f.kind];
  }

  protected value(v: number): string {
    const g = this.graph();
    return g ? formatMetric(v, g.metric, this.metrics()) : '—';
  }

  protected pct(v: number): string {
    const g = this.graph();
    return g && g.focusValue ? formatPct((v / g.focusValue) * 100) : '—';
  }

  protected pp(v: number): string {
    return formatPp(v);
  }

  protected measurement(): string {
    const g = this.graph();
    if (!g) return '';
    const kind = measurementLabel(g.measurement);
    return $localize`:@@profiling.flame.measurementValues:${kind}:kind: values`;
  }

  /** Rectangle index under a pointer event, or null. */
  hitAt(clientX: number, clientY: number): number | null {
    const el = this.surface()?.nativeElement;
    if (!el) return null;
    const box = el.getBoundingClientRect();
    const w = box.width || this.width();
    const depth = depthAt(clientY - box.top, this.model().maxDepth, this.orientation());
    return hitTest(this.model(), (clientX - box.left) / w, depth);
  }

  protected onMove(e: PointerEvent): void {
    const i = this.hitAt(e.clientX, e.clientY);
    this.tip.set(i === null ? null : { x: e.clientX, y: e.clientY, index: i });
  }

  protected onClick(e: MouseEvent): void {
    const i = this.hitAt(e.clientX, e.clientY);
    if (i !== null) this.zoomAt(i);
  }

  zoomAt(index: number): void {
    const r = this.model().rects[index];
    if (!r) return;
    const frame = this.frameOf(r);
    if (frame) this.selected.emit(frame);
    this.zoom.emit(chainTo(this.model(), index).flatMap((i) => {
      const rect = this.model().rects[i];
      return rect ? [rect.frame] : [];
    }));
  }

  protected onKey(e: KeyboardEvent): void {
    const action = flameKey(this.model(), this.current(), e.key, this.orientation());
    if (!action) return;
    e.preventDefault();
    if (action.type === 'move') {
      this.current.set(action.index);
      const r = this.model().rects[action.index];
      if (!r) return;
      const f = this.frameOf(r);
      if (f) this.selected.emit(f);
      this.scrollIntoView(r);
    } else if (action.type === 'zoom') this.zoomAt(action.index);
    else this.zoomOut.emit();
  }

  private scrollIntoView(r: FlameRect): void {
    const scroller = this.surface()?.nativeElement.parentElement;
    if (!scroller) return;
    const y = rowY(r.depth, this.model().maxDepth, this.orientation());
    if (y < scroller.scrollTop || y + FLAME_ROW_H > scroller.scrollTop + scroller.clientHeight) scroller.scrollTop = Math.max(0, y - scroller.clientHeight / 2);
  }
}
