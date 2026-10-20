import { afterNextRender, ChangeDetectionStrategy, Component, computed, DestroyRef, effect, ElementRef, inject, Injector, input, signal, untracked, viewChild } from '@angular/core';
import { RecordView, SourceContext } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { describeError } from '../core/logger';
import { Debouncer } from '../shared/throttle';

/** Lines shown on each side of the event at first, and the steps "More" takes up to the cap (the engine caps at 500). */
export const CONTEXT_STEPS = [10, 50, 200, 500] as const;

/**
 * "Context": the original lines of the file around the selected event, fetched from the engine on demand
 * (only while the section is open). The event's own line is highlighted.
 */
/** The context of a newly selected event is fetched once the selection has rested this long. */
export const CONTEXT_SETTLE_MS = 120;

@Component({
  selector: 'imt-context-section',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="head">
      <button class="toggle section-title" [attr.aria-expanded]="open()" aria-controls="ctx-body" (click)="open.set(!open())" i18n="@@inspector.context.title">
        <span aria-hidden="true">{{ open() ? '▾' : '▸' }}</span> Context
      </button>
      @if (open() && data()) {
        <span class="faint small mono">{{ data()!.sourceName }}</span>
      }
    </div>
    @if (open()) {
      <div id="ctx-body" class="body">
        @if (error(); as e) {
          <div class="err" role="alert">
            <span i18n="@@inspector.context.loadError">Could not load the source lines: {{ e //i18n(ph="error") }}</span>
            <button class="btn" (click)="load()" i18n="@@inspector.context.retry">Retry</button>
          </div>
        } @else if (data(); as d) {
          <div #box class="lines mono" role="region" tabindex="0" aria-label="Source lines around the event" i18n-aria-label="@@inspector.context.linesAria" data-testid="context-lines">
            @for (l of d.lines; track l.line) {
              <div class="line" [class.hit]="l.line === d.line" [attr.aria-current]="l.line === d.line ? 'true' : null">
                <span class="ln">{{ l.line }}</span><span class="lt">{{ l.text }}</span>
              </div>
            }
          </div>
          <div class="actions">
            <button class="btn" [disabled]="!canGrow()" (click)="more()" i18n="@@inspector.context.more">More</button>
            <button class="btn" (click)="copy()">{{ copyLabel() }}</button>
            <span class="faint small" i18n="@@inspector.context.radius">±{{ radius() //i18n(ph="radius") }} lines</span>
          </div>
        } @else {
          <div class="faint pad" role="status" i18n="@@inspector.context.loading">Loading…</div>
        }
      </div>
    }
  `,
  styles: `
    :host { display: block; }
    .head { display: flex; align-items: center; gap: 8px; padding: 12px 14px 6px; }
    .toggle { border: 0; background: transparent; padding: 0; cursor: pointer; text-align: left; }
    .small { font-size: 11px; }
    .body { padding: 0 14px; display: flex; flex-direction: column; gap: 6px; }
    .lines { max-height: 260px; overflow: auto; border: 1px solid var(--border); border-radius: 6px; background: var(--code-bg); font-size: 11px; line-height: 1.55; }
    .lines:focus-visible { outline: 1px solid var(--accent); }
    .line { display: flex; gap: 10px; padding: 0 8px; white-space: pre; color: var(--text-2); }
    .line.hit { background: var(--bg-sel); color: var(--text); box-shadow: inset 2px 0 0 var(--accent); }
    .ln { flex: none; min-width: 5ch; text-align: right; color: var(--muted); user-select: none; }
    .actions { display: flex; align-items: center; gap: 8px; }
    .err { display: flex; align-items: center; justify-content: space-between; gap: 8px; color: var(--danger-text); font-size: 12px; }
    .pad { padding: 4px 0; }
  `,
})
export class ContextSection {
  private readonly backend = inject(ImtBackend);
  private readonly injector = inject(Injector);

  readonly record = input.required<RecordView>();
  protected readonly open = signal(false);
  protected readonly radius = signal<number>(CONTEXT_STEPS[0]);
  protected readonly data = signal<SourceContext | null>(null);
  protected readonly error = signal<string | null>(null);
  protected readonly copyLabel = signal($localize`:@@inspector.context.copy:Copy`);
  protected readonly canGrow = computed(() => this.radius() < (CONTEXT_STEPS[CONTEXT_STEPS.length - 1] ?? 0));
  private readonly box = viewChild<ElementRef<HTMLElement>>('box');
  private seq = 0;
  private shownId = -1;
  /** The selection moved to another event since the last load (its context waits until the selection settles). */
  private moved = false;

  constructor() {
    const settle = new Debouncer(CONTEXT_SETTLE_MS);
    inject(DestroyRef).onDestroy(() => settle.cancel());
    // A different event starts again from the default size; closing the section cancels any request. Opening the
    // section or showing more loads at once; moving the selection (holding an arrow key in the grid) loads once it
    // rests, so passing events are never asked for.
    effect(() => {
      const open = this.open();
      const id = this.record().id;
      const radius = this.radius();
      untracked(() => {
        if (id !== this.shownId) {
          if (this.shownId !== -1) this.moved = true;
          this.shownId = id;
          this.data.set(null);
          if (radius !== CONTEXT_STEPS[0]) {
            this.radius.set(CONTEXT_STEPS[0]);
            return;
          }
        }
        settle.cancel();
        if (!open) {
          this.moved = false;
          this.seq++;
        } else if (this.moved) {
          this.moved = false;
          this.seq++;
          settle.run(() => void this.load());
        } else void this.load();
      });
    });
  }

  protected more(): void {
    const next = CONTEXT_STEPS.find((s) => s > this.radius());
    if (next) this.radius.set(next);
  }

  protected async load(): Promise<void> {
    const mine = ++this.seq;
    const radius = this.radius();
    this.error.set(null);
    try {
      const ctx = await this.backend.sourceContext({ recordId: this.record().id, before: radius, after: radius });
      if (mine !== this.seq) return;
      this.data.set(ctx);
      afterNextRender(() => this.centreHit(), { injector: this.injector });
    } catch (e) {
      if (mine === this.seq) this.error.set(describeError(e));
    }
  }

  private centreHit(): void {
    const box = this.box()?.nativeElement;
    const hit = box?.querySelector<HTMLElement>('.line.hit');
    if (box && hit) box.scrollTop = Math.max(0, hit.offsetTop - box.clientHeight / 2 + hit.offsetHeight / 2);
  }

  protected async copy(): Promise<void> {
    const d = this.data();
    if (!d) return;
    try {
      await navigator.clipboard.writeText(d.lines.map((l) => l.text).join('\n'));
      this.copyLabel.set($localize`:@@inspector.context.copied:Copied`);
    } catch {
      this.copyLabel.set($localize`:@@inspector.context.copyFailed:Copy failed`);
    }
    setTimeout(() => this.copyLabel.set($localize`:@@inspector.context.copy:Copy`), 1500);
  }
}
