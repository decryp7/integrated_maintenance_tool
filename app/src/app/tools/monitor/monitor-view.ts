import { ChangeDetectionStrategy, Component, computed, inject, OnInit, signal } from '@angular/core';
import { Icon } from '../../shared/icon';
import { Switch } from '../../shared/switch';
import { ImtSelect, SelectOption } from '../../settings/themed-select';
import { HelpHint } from '../../ui/help-hint';
import { CpuMode } from '../../core/tools.types';
import { LifecycleService } from '../../lifecycle/lifecycle.service';
import { ToolError } from '../elevation-hint';
import { ToolUnavailable } from '../tool-unavailable';
import { TargetKind } from './monitor-model';
import { MonitorStore } from './monitor.store';
import { MonitorLiveCharts } from './monitor-live-charts';

const KIND_LABEL: Record<TargetKind, string> = {
  name: $localize`:@@tools.monitor.kind.name:Process name (glob)`,
  path: $localize`:@@tools.monitor.kind.path:Executable path (glob)`,
  pid: $localize`:@@tools.monitor.kind.pid:Process id`,
};

/** Process monitor: samples CPU, memory, threads and handles of chosen processes into CLEF files IMT can chart. */
@Component({
  selector: 'imt-monitor-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, HelpHint, ImtSelect, Switch, ToolError, ToolUnavailable, MonitorLiveCharts],
  template: `
    <div class="scroll" role="region" aria-label="Process monitor" i18n-aria-label="@@tools.monitor.region" tabindex="0">
      <p class="intro" i18n="@@tools.monitor.intro">Sample the resource use of processes at an interval. The samples are written as log events and opened here with live tail, so they show up on the timeline, in Metrics and in the Live charts below.</p>
      <imt-tool-unavailable tool="process-monitor" />
      <imt-tool-error [message]="store.error()" (dismissed)="store.dismissError()" />
      @if (store.notice(); as notice) { <div class="note" role="status">{{ notice }}</div> }

      <div class="card status" data-testid="monitor-status" role="status">
        <span class="dot" [style.background]="store.running() ? 'var(--mk-start)' : 'var(--mk-stop)'"></span>
        <strong>{{ runLabel() }}</strong>
        @if (store.dir(); as dir) { <span class="faint ellipsis" [title]="dir" i18n="@@tools.monitor.output">Output: <span class="mono">{{ dir }}</span></span> }
        <span class="spacer"></span>
        <button type="button" class="icon-btn" aria-label="Refresh status" i18n-aria-label="@@tools.monitor.refresh.aria" title="Refresh status" i18n-title="@@tools.monitor.refresh.title" (click)="store.refresh()"><imt-icon name="refresh" [size]="14" /></button>
      </div>

      <imt-monitor-live-charts />

      @if (store.accessDeniedSeen() > 0) {
        <div class="warn" role="alert" data-testid="access-denied-note">
          <ng-container i18n="@@tools.monitor.accessDenied">{store.accessDeniedSeen(), plural, =1 {{{ store.accessDeniedSeen() }} sample} other {{{ store.accessDeniedSeen() }} samples}} could not read some processes (AccessDenied).</ng-container>
          @if (life.canRestartElevated()) {
            <ng-container i18n="@@tools.monitor.adminNeeded">Administrator rights are needed for those.</ng-container>
            <button type="button" class="btn" (click)="life.restartElevated()" i18n="@@tools.monitor.restartAdmin">Restart as administrator</button>
          } @else {
            <ng-container i18n="@@tools.monitor.runAsAdmin">Run IMT as administrator to include them.</ng-container>
          }
        </div>
      }

      <section aria-labelledby="pmon-targets" (input)="edited.set(true)" (change)="edited.set(true)">
        <h2 id="pmon-targets" class="section-title" i18n="@@tools.monitor.targets">Targets <imt-help label="Targets" i18n-label="@@tools.monitor.targets.helpLabel" text="A process is watched when any row matches. Names and paths accept globs such as order*, case-insensitive; a path matches the full executable path." i18n-text="@@tools.monitor.targets.helpText" /></h2>
        @for (t of store.draft().targets; track $index) {
          <div class="inline">
            <imt-select [attr.data-testid]="'target-type-' + $index" [ariaLabel]="targetTypeLabel($index)" [options]="kindOptions" [value]="t.kind" (valueChange)="edited.set(true); store.setTarget($index, { kind: $any($event) })" />
            <input class="input grow" [attr.aria-label]="targetValueLabel($index)" [value]="t.value" placeholder="e.g. OrderService*" i18n-placeholder="@@tools.monitor.target.placeholder" (input)="store.setTarget($index, { value: val($event) })" />
            <button type="button" class="icon-btn" [attr.aria-label]="removeTargetLabel($index)" title="Remove" i18n-title="@@tools.monitor.remove.title" (click)="store.removeTarget($index)"><imt-icon name="close" [size]="13" /></button>
          </div>
        }
        <div><button type="button" class="btn" (click)="store.addTarget()" i18n="@@tools.monitor.addTarget"><imt-icon name="plus" [size]="12" />Add target</button></div>
      </section>

      <section aria-labelledby="pmon-opts" (input)="edited.set(true)" (change)="edited.set(true)">
        <h2 id="pmon-opts" class="section-title" i18n="@@tools.monitor.sampling">Sampling</h2>
        <div class="row">
          <label class="lbl"><ng-container i18n="@@tools.monitor.interval">Interval (ms)</ng-container>
            <input class="input" type="number" min="250" step="250" [value]="store.draft().intervalMs" (input)="store.patchDraft({ intervalMs: +val($event) })" />
          </label>
          <div class="lbl"><ng-container i18n="@@tools.monitor.cpuShownAs">CPU shown as</ng-container>
            <imt-select data-testid="cpu-mode" [ariaLabel]="cpuLabel" [options]="cpuOptions" [value]="store.draft().cpu" (valueChange)="edited.set(true); store.patchDraft({ cpu: $any($event) })" />
          </div>
        </div>
        <imt-switch [checked]="store.draft().includeChildren" (checkedChange)="edited.set(true); store.patchDraft({ includeChildren: $event })"><ng-container i18n="@@tools.monitor.includeChildren">Include child processes</ng-container></imt-switch>
        <imt-switch [checked]="store.draft().dotnet" (checkedChange)="edited.set(true); store.patchDraft({ dotnet: $event })"><ng-container i18n="@@tools.monitor.dotnet">.NET runtime counters (Framework)</ng-container>
          <imt-help label=".NET runtime counters" i18n-label="@@tools.monitor.dotnet.helpLabel" i18n-text="@@tools.monitor.dotnet.helpText" text="Reads GC collections, heap sizes, % time in GC, exceptions/s, lock contention, threads and loaded assemblies of .NET Framework processes. .NET (Core) processes need EventPipe and are reported as not collected." /></imt-switch>
        @if (store.countersAvailable()) {
          <label class="lbl"><ng-container i18n="@@tools.monitor.extraCounters">Extra performance counters (one per line, optional)</ng-container>
            <textarea class="input mono" rows="3" [value]="store.draft().counters" placeholder="\\Process(*)\\IO Data Bytes/sec" (input)="store.patchDraft({ counters: val($event) })"></textarea>
          </label>
        }
        <label class="lbl"><ng-container i18n="@@tools.monitor.outputFolder">Output folder (optional, defaults to IMT's data folder)</ng-container>
          <input class="input mono" [value]="store.draft().dir" (input)="store.patchDraft({ dir: val($event) })" />
        </label>
      </section>

      @if (edited()) {
        @for (p of store.problems(); track p) { <div class="problem">{{ p }}</div> }
      }
      <div class="inline">
        @if (store.running()) {
          <button type="button" class="btn primary" [disabled]="!!store.busy()" (click)="store.stop()" i18n="@@tools.monitor.btn.stop">Stop</button>
        } @else {
          <button type="button" class="btn primary" [disabled]="!!store.busy() || store.problems().length > 0 || !store.canRun()" (click)="store.start()" i18n="@@tools.monitor.btn.start">Start</button>
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
    .lbl { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-xs); color: var(--muted); }
    .problem { color: var(--danger-text); font-size: var(--fs-sm); }
    .note { padding: 6px 10px; border-radius: 6px; border: 1px solid var(--ok); background: var(--bg-3); font-size: var(--fs-sm); }
    .warn { padding: 8px 10px; border-radius: 6px; border: 1px solid var(--lvl-warn); background: var(--bg-3); font-size: var(--fs-sm); display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  `,
})
export class MonitorView implements OnInit {
  /** Validation messages wait until the user has changed the form (Start stays disabled meanwhile). */
  protected readonly edited = signal(false);
  protected readonly store = inject(MonitorStore);
  protected readonly life = inject(LifecycleService);
  protected readonly kindOptions: SelectOption<TargetKind>[] = (['name', 'path', 'pid'] as const).map((k) => ({ value: k, label: KIND_LABEL[k] }));
  protected readonly cpuLabel = $localize`:@@tools.monitor.cpuShownAs.aria:CPU shown as`;
  protected readonly cpuOptions: SelectOption<CpuMode>[] = [
    { value: 'machine', label: $localize`:@@tools.monitor.cpu.machine:Percent of the whole machine` },
    { value: 'core', label: $localize`:@@tools.monitor.cpu.core:Percent of one core (like top)` },
  ];
  protected readonly runLabel = computed(() =>
    this.store.running() ? $localize`:@@tools.monitor.running:Running` : $localize`:@@tools.monitor.stopped:Stopped`,
  );

  protected targetTypeLabel = (i: number) => $localize`:@@tools.monitor.targetTypeN:Target type ${i + 1}:n:`;
  protected targetValueLabel = (i: number) => $localize`:@@tools.monitor.targetValueN:Target value ${i + 1}:n:`;
  protected removeTargetLabel = (i: number) => $localize`:@@tools.monitor.removeTargetN:Remove target ${i + 1}:n:`;

  ngOnInit(): void {
    void this.store.refresh();
  }

  protected val = (e: Event) => (e.target as HTMLInputElement).value;
}
