import { CdkConnectedOverlay, CdkOverlayOrigin, ConnectedPosition } from '@angular/cdk/overlay';
import { ChangeDetectionStrategy, Component, computed, inject, linkedSignal } from '@angular/core';
import { OPEN_SETTINGS, RAISE_CAP } from './service-health.backend';
import { ServiceHealthStore } from './service-health.store';
import { RaiseCap, repairLabel, ServiceHealth, ServiceState } from './service-health.types';

/** "the log store" becomes "Log store". */
export function shortName(label: string): string {
  const bare = label.replace(/^the /i, '');
  return bare.charAt(0).toUpperCase() + bare.slice(1);
}

function stateWord(state: ServiceState): string {
  switch (state) {
    case 'quarantined':
      return $localize`:@@health.state.quarantined:switched off`;
    case 'repairing':
      return $localize`:@@health.state.repairing:repairing`;
    case 'degraded':
      return $localize`:@@health.state.degraded:limited`;
    default:
      return $localize`:@@health.state.down:stopped`;
  }
}

/**
 * The one place a service outage is stated (ADR 0055): a chip in the status bar that appears when any service is not
 * working (a count when several) and opens the plain message, the next step, Details (the technical chain) and the
 * Repair / Try again button. Panels say "Waiting for ..." and link here instead of showing errors of their own.
 */
@Component({
  selector: 'debrief-service-status-chip',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CdkOverlayOrigin, CdkConnectedOverlay],
  host: { style: 'display: contents' },
  template: `
    @if (problems().length) {
      <button
        type="button"
        class="chip"
        data-testid="service-health-chip"
        cdkOverlayOrigin
        #origin="cdkOverlayOrigin"
        aria-haspopup="dialog"
        [attr.aria-expanded]="open()"
        [class.repairing]="allRepairing()"
        (click)="open.set(!open())"
      >{{ text() }}</button>
      <ng-template cdkConnectedOverlay [cdkConnectedOverlayOrigin]="origin" [cdkConnectedOverlayOpen]="open()" [cdkConnectedOverlayPositions]="positions" cdkConnectedOverlayHasBackdrop cdkConnectedOverlayBackdropClass="cdk-overlay-transparent-backdrop" (backdropClick)="open.set(false)" (detach)="open.set(false)" (overlayKeydown)="$event.key === 'Escape' && open.set(false)">
        <div class="pop" role="dialog" aria-label="Services" i18n-aria-label="@@health.chip.label" data-testid="service-health-details">
          @for (s of problems(); track s.service) {
            <section class="svc" [attr.data-testid]="'service-health-' + s.service">
              <strong>{{ s.cause?.message ?? s.label }}</strong>
              @if (s.cause?.hint; as hint) {
                <span class="hint">{{ hint }}</span>
              }
              @if (s.cause?.raiseCap; as cap) {
                @if (canRaise) {
                  <button type="button" class="btn" [attr.data-testid]="'service-health-raise-cap-' + s.service" (click)="raise(cap)" i18n="@@health.chip.raiseCap">Raise the cap…</button>
                }
              }
              @if (s.state === 'repairing') {
                <span class="hint" role="status" i18n="@@health.chip.repairing">Repairing…</span>
              } @else if (s.repair; as action) {
                @if (canRepair(s)) {
                  <button type="button" class="btn" [attr.data-testid]="'service-health-repair-' + s.service" (click)="repair(s)">{{ label(action) }}</button>
                }
              }
              @if (s.cause?.detail; as detail) {
                <details><summary i18n="@@health.chip.technical">Technical details</summary><code class="debrief-selectable">{{ detail }}</code></details>
              }
            </section>
          }
        </div>
      </ng-template>
    }
  `,
  styles: `
    .chip { border: 1px solid var(--lvl-error); background: transparent; color: inherit; border-radius: 9px; height: 18px; padding: 0 8px; cursor: pointer; font-size: var(--fs-xs); font-weight: 600; }
    .chip.repairing { border-color: var(--border-2); }
    .chip:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
    .pop { display: flex; flex-direction: column; gap: 12px; width: 380px; max-width: calc(100vw - 32px); box-sizing: border-box; padding: 12px; background: var(--bg-3); color: var(--text); border: 1px solid var(--border-2); border-radius: 8px; box-shadow: var(--shadow-pop); font-size: var(--fs-md); }
    .svc { display: flex; flex-direction: column; align-items: flex-start; gap: 6px; }
    .svc + .svc { padding-top: 12px; border-top: 1px solid var(--border-2); }
    .hint { color: var(--text-2); line-height: 1.4; }
    details { max-width: 100%; color: var(--muted); font-size: var(--fs-sm); }
    summary { cursor: pointer; }
    code { display: block; margin-top: 6px; white-space: pre-wrap; overflow-wrap: anywhere; font-size: var(--fs-xs); }
  `,
})
export class ServiceStatusChip {
  private readonly store = inject(ServiceHealthStore);
  private readonly raiseCap = inject(RAISE_CAP, { optional: true });
  protected readonly canRaise = !!this.raiseCap;
  private readonly openSettings = inject(OPEN_SETTINGS, { optional: true });
  protected readonly problems = this.store.problems;
  /** Opens by itself when a "Waiting for ..." link asks (`showDetails`). */
  protected readonly open = linkedSignal<number, boolean>({ source: () => this.store.detailsRequested(), computation: (_n, prev) => prev !== undefined });
  protected readonly positions: ConnectedPosition[] = [{ originX: 'end', originY: 'top', overlayX: 'end', overlayY: 'bottom', offsetY: -6 }];
  protected readonly allRepairing = computed(() => this.problems().every((p) => p.state === 'repairing'));
  protected readonly text = computed(() => {
    const ps = this.problems();
    const p = ps[0];
    if (ps.length === 1 && p) {
      return $localize`:@@health.chip.one:${shortName(p.label)}:name:: ${stateWord(p.state)}:state:`;
    }
    return $localize`:@@health.chip.many:${ps.length}:count: services need attention`;
  });

  protected label = repairLabel;

  protected raise(cap: RaiseCap): void {
    void this.raiseCap?.(cap);
  }

  /** Whether this window can run the service's repair: the shell runs every one but `openSettings`, which needs the app's opener. */
  protected canRepair(s: ServiceHealth): boolean {
    return s.repair !== 'openSettings' || (!!this.openSettings && !!s.cause?.settings);
  }

  protected repair(s: ServiceHealth): void {
    const section = s.cause?.settings;
    if (s.repair === 'openSettings') {
      if (section) void this.openSettings?.(section);
      return;
    }
    void this.store.repair(s.service);
  }
}
