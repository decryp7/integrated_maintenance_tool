import { afterNextRender, ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, Injector, untracked, viewChild } from '@angular/core';
import { TourService } from './tour.service';

const POPOVER_WIDTH = 340;
const GAP = 12;

/** Popover position next to the spotlight (below, else above, else centred), kept inside the window. */
export function popoverPosition(spot: { top: number; left: number; width: number; height: number } | null, vw: number, vh: number, ph = 200): { top: number; left: number } {
  const clampLeft = (l: number) => Math.max(GAP, Math.min(l, vw - POPOVER_WIDTH - GAP));
  if (!spot) return { top: Math.max(GAP, Math.round(vh / 2 - ph / 2)), left: clampLeft(Math.round(vw / 2 - POPOVER_WIDTH / 2)) };
  const left = clampLeft(Math.round(spot.left + spot.width / 2 - POPOVER_WIDTH / 2));
  const below = spot.top + spot.height + GAP;
  if (below + ph <= vh) return { top: Math.round(below), left };
  const above = spot.top - GAP - ph;
  if (above >= GAP) return { top: Math.round(above), left };
  // Tall targets: inside the spotlight, near its top.
  return { top: Math.max(GAP, Math.round(Math.min(spot.top + GAP, vh - ph - GAP))), left };
}

/** The guided tour on screen: a spotlight on the step's panel and a keyboard-accessible popover. */
@Component({
  selector: 'imt-tour-overlay',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '(document:keydown)': 'onKey($event)', '(window:resize)': 'tour.remeasure()' },
  template: `
    @if (tour.step(); as step) {
      <div class="shade" [class.dim]="!tour.spot()" aria-hidden="true"></div>
      @if (tour.spot(); as s) {
        <div class="spot" aria-hidden="true" [style.top.px]="s.top - 4" [style.left.px]="s.left - 4" [style.width.px]="s.width + 8" [style.height.px]="s.height + 8"></div>
      }
      <div #box class="pop" role="dialog" aria-modal="false" aria-labelledby="imt-tour-title" aria-describedby="imt-tour-text" data-testid="tour" [style.top.px]="pos().top" [style.left.px]="pos().left">
        <div class="count" i18n="@@onboarding.overlay.count">Step {{ tour.index() + 1 }} of {{ tour.steps.length }}</div>
        <h2 id="imt-tour-title">{{ step.title }}</h2>
        <p id="imt-tour-text">{{ step.text }}</p>
        <div class="foot">
          <button type="button" class="btn ghost" (click)="tour.skip()" i18n="@@onboarding.overlay.skip">Skip tour</button>
          <span class="spacer"></span>
          <button type="button" class="btn" [disabled]="tour.isFirst()" (click)="tour.back()" i18n="@@onboarding.overlay.back">Back</button>
          <button #next type="button" class="btn primary" (click)="tour.next()">{{ nextLabel() }}</button>
        </div>
      </div>
      <p class="sr-only" aria-live="polite" i18n="@@onboarding.overlay.live">Tour step {{ tour.index() + 1 }} of {{ tour.steps.length }}: {{ step.title }}. {{ step.text }}</p>
    }
  `,
  styles: `
    .shade { position: fixed; inset: 0; z-index: 180; pointer-events: none; }
    .shade.dim { background: var(--scrim); }
    .spot { position: fixed; z-index: 181; border-radius: 8px; border: 2px solid var(--accent); box-shadow: 0 0 0 9999px var(--scrim); pointer-events: none; transition: all 0.2s ease; }
    .pop { position: fixed; z-index: 182; width: ${POPOVER_WIDTH}px; display: flex; flex-direction: column; gap: 8px; padding: 14px 16px; border-radius: 10px; background: var(--bg-2); border: 1px solid var(--border-3); box-shadow: var(--shadow); color: var(--text); }
    .count { font-size: var(--fs-xs); color: var(--muted); }
    h2 { margin: 0; font-size: var(--fs-lg); font-weight: 600; }
    p { margin: 0; font-size: var(--fs-md); line-height: 1.45; color: var(--text-2); }
    .foot { display: flex; align-items: center; gap: 6px; margin-top: 4px; }
    .spacer { flex: 1; }
    @media (prefers-reduced-motion: reduce) { .spot { transition: none; } }
  `,
})
export class TourOverlay {
  protected readonly tour = inject(TourService);
  private readonly injector = inject(Injector);
  private readonly nextButton = viewChild<ElementRef<HTMLButtonElement>>('next');
  private readonly box = viewChild<ElementRef<HTMLElement>>('box');
  private returnFocus: HTMLElement | null = null;

  protected readonly nextLabel = computed(() => (this.tour.isLast() ? $localize`:@@onboarding.overlay.finish:Finish` : $localize`:@@onboarding.overlay.next:Next`));
  protected readonly pos = computed(() => popoverPosition(this.tour.spot(), window.innerWidth, window.innerHeight, this.box()?.nativeElement.offsetHeight || 200));

  constructor() {
    // Focus moves into the popover on every step and returns to where it was when the tour ends.
    effect(() => {
      const index = this.tour.index();
      this.tour.spot();
      untracked(() => {
        if (index < 0) {
          this.returnFocus?.focus?.();
          this.returnFocus = null;
          return;
        }
        if (!this.returnFocus) this.returnFocus = document.activeElement as HTMLElement | null;
        afterNextRender(() => this.nextButton()?.nativeElement.focus(), { injector: this.injector });
      });
    });
  }

  protected onKey(e: KeyboardEvent): void {
    if (!this.tour.active()) return;
    const onButton = (e.target as HTMLElement | null)?.tagName === 'BUTTON';
    if (e.key === 'Escape') {
      e.preventDefault();
      this.tour.skip();
    } else if (e.key === 'ArrowRight' || (e.key === 'Enter' && !onButton)) {
      e.preventDefault();
      this.tour.next();
    } else if (e.key === 'ArrowLeft') {
      e.preventDefault();
      this.tour.back();
    }
  }
}
