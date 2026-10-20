import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { formatCount } from '../core/format';
import { LiveFeed } from './live-feed';

/** "N new events" over the grid while live tail runs but the grid is not following; click to show them. */
@Component({
  selector: 'imt-new-events-pill',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (feed.newEvents() > 0) {
      <button type="button" class="pill" data-testid="new-events" (click)="feed.showNewEvents()" i18n="@@live.newEvents.pill">
        <span aria-hidden="true">↑</span> {feed.newEvents(), plural, =1 {{{ fmt(feed.newEvents()) }} new event — show} other {{{ fmt(feed.newEvents()) }} new events — show}}
      </button>
    }
  `,
  styles: `
    :host { position: absolute; top: 34px; left: 50%; transform: translateX(-50%); z-index: 4; }
    .pill { height: 24px; padding: 0 12px; border-radius: 12px; border: 1px solid var(--accent); background: var(--bg-3); color: var(--text); font: inherit; font-size: var(--fs-sm); cursor: pointer; box-shadow: var(--shadow); white-space: nowrap; }
    .pill:hover { background: var(--bg-hover); }
  `,
})
export class NewEventsPill {
  protected readonly feed = inject(LiveFeed);
  protected readonly fmt = formatCount;
}
