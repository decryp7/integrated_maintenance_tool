import { ChangeDetectionStrategy, Component, inject, OnInit } from '@angular/core';
import { RunStatus } from '../core/agent.types';
import { formatDuration } from '../core/format';
import { Icon } from '../shared/icon';
import { RunsStore } from './runs.store';
import { formatBytes } from './schedule-model';

const STATUS_LABEL: Record<RunStatus, string> = {
  running: $localize`:@@schedules.runs.status.running:Running`,
  ok: $localize`:@@schedules.runs.status.ok:OK`,
  partial: $localize`:@@schedules.runs.status.partial:Partial`,
  failed: $localize`:@@schedules.runs.status.failed:Failed`,
  stopped: $localize`:@@schedules.runs.status.stopped:Stopped`,
  incomplete: $localize`:@@schedules.runs.status.incomplete:Incomplete`,
};

/** "Runs": the folders the headless agent wrote, newest first; open one in a new window. */
@Component({
  selector: 'imt-runs-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    <div class="scroll" role="region" aria-label="Collection runs" i18n-aria-label="@@schedules.runs.region" tabindex="0">
      <div class="inline">
        <span class="faint ellipsis">
          @if (store.outputDir(); as dir) { <ng-container i18n="@@schedules.runs.output">Output: <span class="mono">{{ dir }}</span></ng-container> } @else { <ng-container i18n="@@schedules.runs.runsOf">Runs of the collection schedules.</ng-container> }
        </span>
        <span class="spacer"></span>
        <button type="button" class="btn" [disabled]="store.loading()" (click)="store.refresh()" i18n="@@schedules.runs.refresh"><imt-icon name="refresh" [size]="12" />Refresh</button>
      </div>
      @if (store.error(); as e) { <div class="err" role="alert">{{ e }}</div> }
      @if (store.notice(); as n) { <div class="note" role="status">{{ n }}</div> }

      @if (store.runs().length) {
        <table class="runs" aria-label="Runs" i18n-aria-label="@@schedules.runs.table">
          <thead>
            <tr><th scope="col" i18n="@@schedules.runs.col.started">Started</th><th scope="col" i18n="@@schedules.runs.col.schedule">Schedule</th><th scope="col" i18n="@@schedules.runs.col.status">Status</th><th scope="col" i18n="@@schedules.runs.col.why">Why</th><th scope="col" i18n="@@schedules.runs.col.duration">Duration</th><th scope="col" i18n="@@schedules.runs.col.size">Size</th><th scope="col"><span class="sr-only" i18n="@@schedules.runs.col.actions">Actions</span></th></tr>
          </thead>
          <tbody>
            @for (r of store.runs(); track r.id) {
              <tr [attr.data-testid]="'run-' + r.id">
                <td class="mono">{{ when(r.startedMs) }}</td>
                <td>{{ r.schedule }}</td>
                <td><span class="badge" [attr.data-status]="r.status">{{ label[r.status] }}</span></td>
                <td class="why" [title]="r.error ?? r.reason">{{ r.reason }}@if (r.error) {<br /><span class="problem">{{ r.error }}</span>}</td>
                <td>{{ duration(r.durationMs) }}</td>
                <td>{{ bytes(r.totalBytes) }}</td>
                <td class="acts">
                  <button type="button" class="btn" [attr.aria-expanded]="store.selected() === r.id" [attr.aria-label]="detailsLabel(r.id)" (click)="store.toggle(r.id)" i18n="@@schedules.runs.details">Details</button>
                  <button type="button" class="btn" [attr.aria-label]="openLabel(r.id)" (click)="store.open(r.id)" i18n="open a run|@@schedules.runs.open">Open</button>
                </td>
              </tr>
              @if (store.selected() === r.id) {
                <tr class="details">
                  <td colspan="7">
                    @if (store.manifest(); as m) {
                      <div class="manifest" data-testid="run-manifest">
                        <div class="faint" i18n="@@schedules.runs.manifestHead">Host {{ m.host }} · agent {{ m.agentVersion }} · {{ m.startedAt }}</div>
                        <ul>
                          @for (a of m.actions; track $index) {
                            <li>
                              <strong>{{ a.kind }}</strong> — {{ a.status }} · {{ duration(a.ms) }} · {{ bytes(a.bytes) }}
                              @if (a.message) { <span class="faint"> · {{ a.message }}</span> }
                              @for (o of a.outputs; track o) { <div class="mono faint">{{ o }}</div> }
                            </li>
                          }
                        </ul>
                        @for (e of m.errors; track e) { <div class="problem">{{ e }}</div> }
                      </div>
                    } @else {
                      <span class="faint" role="status" i18n="@@schedules.runs.loading">Loading…</span>
                    }
                  </td>
                </tr>
              }
            }
          </tbody>
        </table>
        @if (store.truncated()) { <p class="faint" i18n="@@schedules.runs.truncated">{{ store.truncated() }} older runs are not listed.</p> }
      } @else if (!store.loading() && !store.error()) {
        <p class="faint" i18n="@@schedules.runs.none">No runs yet. They appear here once the agent has run a schedule (or after <span class="mono">imt-agent once</span>).</p>
      }
    </div>
  `,
  styles: `
    :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .scroll { flex: 1; min-height: 0; overflow: auto; padding: 14px 18px; display: flex; flex-direction: column; gap: 10px; }
    .scroll:focus-visible { outline-offset: -2px; }
    .inline { display: flex; gap: 8px; align-items: center; }
    .spacer { flex: 1; }
    .runs { border-collapse: collapse; width: 100%; font-size: var(--fs-sm); }
    th { text-align: left; font-weight: 600; color: var(--text-2); border-bottom: 1px solid var(--border-3); padding: 4px 6px; }
    td { border-bottom: 1px solid var(--border-3); padding: 4px 6px; vertical-align: top; }
    .why { max-width: 360px; overflow: hidden; text-overflow: ellipsis; }
    .acts { white-space: nowrap; display: flex; gap: 4px; }
    .badge { padding: 1px 6px; border-radius: 8px; border: 1px solid var(--border-3); background: var(--bg-3); }
    .badge[data-status='ok'] { border-color: var(--ok); }
    .badge[data-status='failed'], .badge[data-status='incomplete'] { border-color: var(--danger-text); color: var(--danger-text); }
    .badge[data-status='partial'], .badge[data-status='stopped'] { border-color: var(--lvl-warn); }
    .manifest ul { margin: 4px 0; padding-left: 18px; }
    .problem { color: var(--danger-text); }
    .err { padding: 6px 10px; border-radius: 6px; border: 1px solid var(--danger-text); background: var(--bg-3); color: var(--danger-text); font-size: var(--fs-sm); }
    .note { padding: 6px 10px; border-radius: 6px; border: 1px solid var(--ok); background: var(--bg-3); font-size: var(--fs-sm); }
    .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
  `,
})
export class RunsView implements OnInit {
  protected readonly store = inject(RunsStore);
  protected readonly label = STATUS_LABEL;
  protected detailsLabel = (id: string) => $localize`:@@schedules.runs.detailsOf:Details of ${id}:id:`;
  protected openLabel = (id: string) => $localize`:@@schedules.runs.openInWindow:Open ${id}:id: in a new window`;

  ngOnInit(): void {
    void this.store.refresh();
  }

  protected when(ms: number): string {
    const d = new Date(ms);
    const p = (n: number) => String(n).padStart(2, '0');
    return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
  }

  protected duration = formatDuration;
  protected bytes = formatBytes;
}
