import { ChangeDetectionStrategy, Component, computed, inject, input, output } from '@angular/core';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { isAccessDenied } from './tool-errors';

/** Shows an inline error and, when it was caused by missing rights, the way forward. */
@Component({
  selector: 'imt-tool-error',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (message(); as text) {
      <div class="err" role="alert">
        <span class="text">{{ text }}</span>
        @if (denied()) {
          @if (life.canRestartElevated()) {
            <button type="button" class="btn" (click)="life.restartElevated()" i18n="@@tools.elevation.restart">Restart as administrator</button>
          } @else if (!life.elevated()) {
            <span class="faint" i18n="@@tools.elevation.runAsAdmin">Run IMT as administrator and try again.</span>
          }
        }
        @if (retry()) {
          <button type="button" class="btn" (click)="retried.emit()" i18n="@@tools.elevation.retry">Retry</button>
        }
        <button type="button" class="btn ghost" (click)="dismissed.emit()" i18n="@@tools.elevation.dismiss">Dismiss</button>
      </div>
    }
  `,
  styles: `
    .err { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; padding: 8px 10px; border-radius: 6px; border: 1px solid var(--lvl-error); background: var(--bg-3); color: var(--text); font-size: var(--fs-sm); }
    .text { flex: 1; min-width: 200px; color: var(--danger-text); }
  `,
})
export class ToolError {
  protected readonly life = inject(LifecycleService);
  readonly message = input<string | null>(null);
  /** Show a Retry button (the host listens to `retried`). */
  readonly retry = input(false);
  protected readonly denied = computed(() => isAccessDenied(this.message()));
  readonly dismissed = output<void>();
  readonly retried = output<void>();
}
