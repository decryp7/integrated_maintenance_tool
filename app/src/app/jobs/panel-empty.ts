import { ChangeDetectionStrategy, Component, input } from '@angular/core';
import { Icon } from '../shared/icon';

/** A designed empty state for a pane: a 32 px icon, a short title and one sentence. */
@Component({
  selector: 'imt-panel-empty',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    <imt-icon [name]="icon()" [size]="32" />
    <h3 class="title">{{ title() }}</h3>
    <p class="text"><ng-content /></p>
  `,
  styles: `
    :host { display: flex; flex-direction: column; align-items: center; gap: 6px; padding: 28px 16px; text-align: center; color: var(--muted); }
    .title { margin: 0; font-size: var(--fs-lg); font-weight: 600; color: var(--text); text-transform: none; letter-spacing: normal; }
    .text { margin: 0; max-width: 360px; line-height: 1.4; }
  `,
})
export class PanelEmpty {
  readonly icon = input.required<string>();
  readonly title = input.required<string>();
}
