import { ChangeDetectionStrategy, Component, inject, OnInit } from '@angular/core';
import { Icon } from '../../shared/icon';
import { HelpHint } from '../../ui/help-hint';
import { CpuMode } from '../../core/tools.types';
import { LifecycleService } from '../../lifecycle/lifecycle.service';
import { ToolError } from '../elevation-hint';
import { TargetKind } from './monitor-model';
import { MonitorStore } from './monitor.store';

const KIND_LABEL: Record<TargetKind, string> = { name: 'Process name (glob)', path: 'Executable path (glob)', pid: 'Process id' };

/** Process monitor: samples CPU, memory, threads and handles of chosen processes into CLEF files IMT can chart. */
@Component({
  selector: 'imt-monitor-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, HelpHint, ToolError],
  template: `
    <div class="scroll" role="region" aria-label="Process monitor" tabindex="0">
      <p class="intro">Sample the resource use of processes at an interval. The samples are written as log events and opened here with live tail, so they show up on the timeline and in Metrics.</p>
      <imt-tool-error [message]="store.error()" (dismissed)="store.dismissError()" />
      @if (store.notice(); as notice) { <div class="note" role="status">{{ notice }}</div> }

      <div class="card status" data-testid="monitor-status" role="status">
        <span class="dot" [style.background]="store.running() ? 'var(--mk-start)' : 'var(--mk-stop)'"></span>
        <strong>{{ store.running() ? 'Running' : 'Stopped' }}</strong>
        @if (store.dir(); as dir) { <span class="faint ellipsis" [title]="dir">Output: <span class="mono">{{ dir }}</span></span> }
        <span class="spacer"></span>
        <button type="button" class="icon-btn" aria-label="Refresh status" title="Refresh status" (click)="store.refresh()"><imt-icon name="refresh" [size]="14" /></button>
      </div>

      @if (store.accessDeniedSeen() > 0) {
        <div class="warn" role="alert" data-testid="access-denied-note">
          {{ store.accessDeniedSeen() }} sample{{ store.accessDeniedSeen() === 1 ? '' : 's' }} could not read some processes (AccessDenied).
          @if (life.canRestartElevated()) {
            Administrator rights are needed for those.
            <button type="button" class="btn" (click)="life.restartElevated()">Restart as administrator</button>
          } @else {
            Run IMT as administrator to include them.
          }
        </div>
      }

      <section aria-labelledby="pmon-targets">
        <h2 id="pmon-targets" class="section-title">Targets <imt-help label="Targets" text="A process is watched when any row matches. Names and paths accept globs such as order*, case-insensitive; a path matches the full executable path." /></h2>
        @for (t of store.draft().targets; track $index) {
          <div class="inline">
            <select class="select" [attr.aria-label]="'Target type ' + ($index + 1)" [value]="t.kind" (change)="store.setTarget($index, { kind: kind($event) })">
              @for (k of kinds; track k) { <option [value]="k">{{ label[k] }}</option> }
            </select>
            <input class="input grow" [attr.aria-label]="'Target value ' + ($index + 1)" [value]="t.value" placeholder="e.g. OrderService*" (input)="store.setTarget($index, { value: val($event) })" />
            <button type="button" class="icon-btn" [attr.aria-label]="'Remove target ' + ($index + 1)" title="Remove" (click)="store.removeTarget($index)"><imt-icon name="close" [size]="13" /></button>
          </div>
        }
        <div><button type="button" class="btn" (click)="store.addTarget()"><imt-icon name="plus" [size]="12" />Add target</button></div>
      </section>

      <section aria-labelledby="pmon-opts">
        <h2 id="pmon-opts" class="section-title">Sampling</h2>
        <div class="row">
          <label class="lbl">Interval (ms)
            <input class="input" type="number" min="250" step="250" [value]="store.draft().intervalMs" (input)="store.patchDraft({ intervalMs: +val($event) })" />
          </label>
          <label class="lbl">CPU shown as
            <select class="select" [value]="store.draft().cpu" (change)="store.patchDraft({ cpu: cpu($event) })">
              <option value="machine">Percent of the whole machine</option>
              <option value="core">Percent of one core (like top)</option>
            </select>
          </label>
        </div>
        <label class="check"><input type="checkbox" [checked]="store.draft().includeChildren" (change)="store.patchDraft({ includeChildren: checked($event) })" /> Include child processes</label>
        @if (store.countersAvailable()) {
          <label class="lbl">Extra performance counters (one per line, optional)
            <textarea class="input mono" rows="3" [value]="store.draft().counters" placeholder="\\Process(*)\\IO Data Bytes/sec" (input)="store.patchDraft({ counters: val($event) })"></textarea>
          </label>
        }
        <label class="lbl">Output folder (optional, defaults to IMT's data folder)
          <input class="input mono" [value]="store.draft().dir" (input)="store.patchDraft({ dir: val($event) })" />
        </label>
      </section>

      @for (p of store.problems(); track p) { <div class="problem">{{ p }}</div> }
      <div class="inline">
        @if (store.running()) {
          <button type="button" class="btn primary" [disabled]="!!store.busy()" (click)="store.stop()">Stop</button>
        } @else {
          <button type="button" class="btn primary" [disabled]="!!store.busy() || store.problems().length > 0" (click)="store.start()">Start</button>
        }
        @if (store.busy(); as busy) { <span class="faint" role="status">{{ busy }}</span> }
      </div>
    </div>
  `,
  styles: `
    :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .scroll { flex: 1; min-height: 0; overflow: auto; padding: 14px 18px; display: flex; flex-direction: column; gap: 12px; max-width: 780px; }
    .scroll:focus-visible { outline-offset: -2px; }
    section { display: flex; flex-direction: column; gap: 8px; }
    h2 { margin: 0; display: flex; align-items: center; gap: 6px; }
    .intro { margin: 0; color: var(--text-2); line-height: 1.5; }
    .card { padding: 8px 12px; border: 1px solid var(--border-3); border-radius: 8px; background: var(--bg-2); }
    .status { display: flex; align-items: center; gap: 10px; }
    .spacer { flex: 1; }
    .inline { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
    .grow { flex: 1; }
    .row { display: flex; gap: 12px; flex-wrap: wrap; }
    .row .lbl { flex: 1; min-width: 160px; }
    .lbl { display: flex; flex-direction: column; gap: 4px; font-size: 11px; color: var(--muted); }
    .check { display: inline-flex; gap: 6px; align-items: center; color: var(--text-2); }
    .problem { color: var(--danger-text); font-size: 12px; }
    .note { padding: 6px 10px; border-radius: 6px; border: 1px solid var(--ok); background: var(--bg-3); font-size: 12px; }
    .warn { padding: 8px 10px; border-radius: 6px; border: 1px solid var(--lvl-warn); background: var(--bg-3); font-size: 12px; display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  `,
})
export class MonitorView implements OnInit {
  protected readonly store = inject(MonitorStore);
  protected readonly life = inject(LifecycleService);
  protected readonly kinds: TargetKind[] = ['name', 'path', 'pid'];
  protected readonly label = KIND_LABEL;

  ngOnInit(): void {
    void this.store.refresh();
  }

  protected val = (e: Event) => (e.target as HTMLInputElement).value;
  protected checked = (e: Event) => (e.target as HTMLInputElement).checked;
  protected kind = (e: Event) => this.val(e) as TargetKind;
  protected cpu = (e: Event) => this.val(e) as CpuMode;
}
