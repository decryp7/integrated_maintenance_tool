import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { LiveTail } from '../workspace/live-tail';
import { CUSTOM_MINUTES, LiveFollow, WINDOW_PRESETS, WindowPreset } from './live-follow.store';

/**
 * Timeline header controls while live tail is on: the Follow toggle, the sliding window, and, after a manual
 * zoom or pan, the "Paused — Resume following" chip.
 */
@Component({
  selector: 'imt-live-window-controls',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (live.enabled()) {
      <div class="live-ctl" role="group" aria-label="Follow live events" i18n-aria-label="@@live.windowControls.group">
        @if (follow.paused() && follow.follow()) {
          <button type="button" class="paused" data-testid="resume-following" (click)="follow.resume()" i18n="@@live.windowControls.resume">
            <span aria-hidden="true">⏸</span> Paused — Resume following
          </button>
        } @else {
          <button
            type="button"
            class="toggle"
            data-testid="follow-toggle"
            [attr.aria-pressed]="follow.follow()"
            title="Keep the newest events in view as they arrive" i18n-title="@@live.windowControls.followTitle"
            (click)="follow.setFollow(!follow.follow())"
            i18n="@@live.windowControls.follow"
          ><span class="dot" aria-hidden="true"></span>Follow</button>
        }
        <label class="win">
          <span class="sr" i18n="@@live.windowControls.window">Live window</span>
          <select class="select" aria-label="Live window" i18n-aria-label="@@live.windowControls.windowAria" (change)="setPreset($any($event.target).value)">
            @for (p of presets; track p.id) { <option [value]="p.id" [selected]="p.id === follow.preset()">{{ p.label }}</option> }
          </select>
        </label>
        @if (follow.preset() === 'custom') {
          <label class="custom">
            <input
              class="input mono"
              type="number"
              [min]="limits.min"
              [max]="limits.max"
              aria-label="Custom window in minutes" i18n-aria-label="@@live.windowControls.customAria"
              [value]="follow.customMinutes()"
              (change)="setCustom($any($event.target).value)"
            /><span class="faint" i18n="@@live.windowControls.minutes">min</span>
          </label>
        }
      </div>
    }
  `,
  styles: `
    :host { display: contents; }
    .live-ctl { display: flex; align-items: center; gap: 6px; flex: none; }
    .toggle, .paused { display: inline-flex; align-items: center; gap: 6px; height: 22px; padding: 0 9px; border-radius: 11px; border: 1px solid var(--border-2); background: transparent; color: var(--text-2); font: inherit; font-size: var(--fs-sm); cursor: pointer; white-space: nowrap; }
    .toggle[aria-pressed='true'] { border-color: var(--accent); background: var(--accent-soft); color: var(--text); }
    .dot { width: 7px; height: 7px; border-radius: 50%; border: 1px solid currentColor; }
    .toggle[aria-pressed='true'] .dot { background: var(--accent); border-color: var(--accent); }
    .paused { border-color: var(--warn-text); color: var(--text); background: var(--bg-3); }
    .paused:hover, .toggle:hover { background: var(--bg-hover); }
    .select { height: 22px; font-size: var(--fs-sm); padding: 0 4px; }
    .custom { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-sm); }
    .custom .input { width: 64px; height: 22px; }
    .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
  `,
})
export class LiveWindowControls {
  protected readonly live = inject(LiveTail);
  protected readonly follow = inject(LiveFollow);
  protected readonly presets = WINDOW_PRESETS;
  protected readonly limits = CUSTOM_MINUTES;

  protected setPreset(id: WindowPreset): void {
    this.follow.setPreset(id);
  }

  protected setCustom(text: string): void {
    const n = Number(text);
    if (Number.isFinite(n) && n > 0) this.follow.setCustomMinutes(n);
  }
}
