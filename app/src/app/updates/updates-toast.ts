import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { UpdatesCommands } from './updates.commands';
import { UpdatesStore } from './updates.store';

/** "IMT x.y is available — View" after an automatic check. Mounted once over the app. */
@Component({
  selector: 'imt-updates-toast',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (store.notice(); as version) {
      <div class="toast" role="status" aria-live="polite" data-testid="updates-toast">
        <span i18n="@@updates.toast.text"><strong>IMT {{ version }} is available</strong> — <button type="button" class="link" (click)="view()">View</button></span>
        <button type="button" class="x" aria-label="Dismiss" i18n-aria-label="@@updates.toast.dismiss" (click)="store.dismissNotice()">×</button>
      </div>
    }
  `,
  styles: `
    :host { position: fixed; right: 16px; bottom: 96px; z-index: 2000; }
    .toast { position: relative; max-width: 360px; padding: 10px 32px 10px 12px; border: 1px solid var(--border-2); border-left: 3px solid var(--accent); border-radius: 8px; background: var(--bg-3); color: var(--text); box-shadow: var(--shadow-pop); font-size: var(--fs-md); }
    .link { all: unset; cursor: pointer; color: var(--accent-text); text-decoration: underline; }
    .link:focus-visible, .x:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
    .x { all: unset; position: absolute; top: 6px; right: 8px; cursor: pointer; color: var(--text-2); font-size: var(--fs-lg); line-height: 1; }
  `,
})
export class UpdatesToast {
  protected readonly store = inject(UpdatesStore);
  private readonly commands = inject(UpdatesCommands);

  protected view(): void {
    this.store.dismissNotice();
    this.commands.open();
  }
}
