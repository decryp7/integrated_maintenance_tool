import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { plural } from '../i18n/locale';
import { ClickOutside } from '../shared/click-outside';
import { WindowingService } from './windowing.service';
import { BackgroundTask } from './windowing.types';

/** Status-bar button listing the work that keeps running in the background, with Stop buttons. */
@Component({
  selector: 'imt-background-work',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ClickOutside],
  template: `
    @if (windowing.runningCount() > 0) {
      <span class="wrap" (imtClickOutside)="open.set(false)">
        <button
          type="button"
          class="trigger"
          aria-label="Background work"
          i18n-aria-label="@@windowing.backgroundWork.trigger"
          aria-haspopup="dialog"
          [attr.aria-expanded]="open()"
          [title]="runningTitle()"
          (click)="open.set(!open())"
        >
          <span class="spin" aria-hidden="true"></span>{{ windowing.runningCount() }}
        </button>
        @if (open()) {
          <div class="popover panel" role="dialog" aria-label="Background work running" i18n-aria-label="@@windowing.backgroundWork.popover">
            <h3 i18n="@@windowing.backgroundWork.heading">Background work</h3>
            <ul>
              @for (task of windowing.tasks(); track task.id) {
                <li>
                  <div class="row">
                    <span class="ellipsis" [title]="task.label">{{ task.label }}</span>
                    <button type="button" class="btn" [disabled]="!task.stoppable" [attr.aria-label]="stopLabel(task.label)" (click)="stop(task)" i18n="@@windowing.backgroundWork.stop">Stop</button>
                  </div>
                  @if (task.progress !== null) {
                    <progress max="1" [value]="task.progress" [attr.aria-label]="progressLabel(task.label)"></progress>
                  }
                </li>
              }
            </ul>
          </div>
        }
      </span>
    }
  `,
  styles: `
    :host { display: contents; }
    .trigger { display: inline-flex; align-items: center; gap: 6px; height: 18px; padding: 0 8px; border-radius: 9px; border: 1px solid currentColor; background: transparent; color: inherit; cursor: pointer; font-size: 11px; font-weight: 600; }
    .trigger:hover { background: color-mix(in srgb, var(--accent-ink) 18%, transparent); }
    .spin { width: 8px; height: 8px; border-radius: 50%; border: 1.5px solid currentColor; border-top-color: transparent; animation: spin 1.2s linear infinite; }
    @keyframes spin { to { transform: rotate(360deg); } }
    @media (prefers-reduced-motion: reduce) { .spin { animation: none; } }
    .panel { position: fixed; inset-inline-end: 12px; bottom: 30px; width: 320px; padding: 10px 12px; z-index: 60; color: var(--text); font-size: 12px; white-space: normal; }
    h3 { margin: 0 0 8px; font-size: 12px; font-weight: 600; }
    ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
    .row { display: flex; align-items: center; justify-content: space-between; gap: 8px; min-width: 0; }
    progress { width: 100%; height: 6px; margin-top: 4px; accent-color: var(--accent); }
  `,
})
export class BackgroundWork {
  protected readonly windowing = inject(WindowingService);
  protected readonly open = signal(false);

  protected runningTitle(): string {
    const n = this.windowing.runningCount();
    return plural(n, {
      one: $localize`:@@windowing.backgroundWork.running.one:${n}:count: background task running`,
      other: $localize`:@@windowing.backgroundWork.running.other:${n}:count: background tasks running`,
    });
  }

  protected stopLabel(task: string): string {
    return $localize`:@@windowing.backgroundWork.stopLabel:Stop ${task}:task:`;
  }

  protected progressLabel(task: string): string {
    return $localize`:@@windowing.backgroundWork.progressLabel:${task}:task: progress`;
  }

  protected async stop(task: BackgroundTask): Promise<void> {
    await this.windowing.stopTask(task.id);
    if (this.windowing.runningCount() === 0) this.open.set(false);
  }
}
