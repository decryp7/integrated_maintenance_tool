import { CdkListbox, CdkOption } from '@angular/cdk/listbox';
import { ChangeDetectionStrategy, Component, computed, inject, OnInit } from '@angular/core';
import { DotnetCaptureKind, DotnetCaptureStatus } from '../../core/api.types';
import { ToolError } from '../../tools/elevation-hint';
import { DIALOG_REF } from '../../ui/dialog.service';
import { formatBytes, formatNs } from '../profiling-format';
import { DotnetCaptureStore, isRunning } from './dotnet-capture.store';

interface KindChoice {
  kind: DotnetCaptureKind;
  label: string;
  help: string;
}

/** What each capture records, its cost, and what it shows (plain words for the dialog). */
export const KIND_CHOICES: KindChoice[] = [
  {
    kind: 'cpu',
    label: $localize`:@@profiling.dotnet.kind.cpu:CPU and waits`,
    help: $localize`:@@profiling.dotnet.kind.cpu.help:Samples every thread about 1000 times a second, plus GC, exceptions, lock contention, JIT and (on .NET Framework) where threads block. Low overhead; shows where time goes and why threads wait.`,
  },
  {
    kind: 'allocations',
    label: $localize`:@@profiling.dotnet.kind.allocations:Allocations`,
    help: $localize`:@@profiling.dotnet.kind.allocations.help:Like CPU, plus one allocation sample per ~100 KB allocated, with its type and call stack. Slightly more overhead; shows what allocates and drives garbage collection.`,
  },
  {
    kind: 'heap',
    label: $localize`:@@profiling.dotnet.kind.heap:Heap snapshot`,
    help: $localize`:@@profiling.dotnet.kind.heap.help:Makes the runtime run one full garbage collection and describe every object: types, sizes, references and roots. Pauses the process for the collection (a second or more on big heaps); shows what keeps memory alive.`,
  },
];

const STATE_LABEL: Record<DotnetCaptureStatus['state'], string> = {
  recording: $localize`:@@profiling.dotnet.state.recording:Recording`,
  stopping: $localize`:@@profiling.dotnet.state.stopping:Finishing (collecting method names)…`,
  done: $localize`:@@profiling.dotnet.state.done:Done`,
  failed: $localize`:@@profiling.dotnet.state.failed:Failed`,
  cancelled: $localize`:@@profiling.dotnet.state.cancelled:Cancelled`,
};

/**
 * "Profile .NET process…": pick a running .NET process, choose what to record, then watch it record. Nothing is
 * loaded into the process: .NET Framework is recorded with ETW (administrator rights), .NET Core with EventPipe.
 * A finished capture opens in the profiling views; captures also run as Jobs, so the dialog may be closed.
 */
@Component({
  selector: 'imt-dotnet-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CdkListbox, CdkOption, ToolError],
  template: `
    <div class="bar">
      <label class="sr-only" for="dotnet-filter" i18n="@@profiling.dotnet.filter.label">Filter processes</label>
      <input id="dotnet-filter" class="input" type="search" maxlength="200" placeholder="Filter by name, id or command line" i18n-placeholder="@@profiling.dotnet.filter.placeholder" [value]="store.filter()" (input)="store.setFilter(asValue($event))" />
      <button type="button" class="btn" (click)="store.refresh()" [disabled]="store.loading()" i18n="@@profiling.dotnet.refresh">Refresh</button>
    </div>
    @if (store.loading() && !store.processes().length) {
      <p class="muted" i18n="@@profiling.dotnet.loading">Looking for .NET processes…</p>
    } @else if (!store.visible().length) {
      <p class="muted" i18n="@@profiling.dotnet.none">No .NET process found. Start the application, then Refresh. Processes of other users are listed only when IMT runs as administrator.</p>
    } @else {
      <ul class="procs" cdkListbox [cdkListboxValue]="selectedValue()" (cdkListboxValueChange)="onSelect($event.value)" aria-label="NET processes" i18n-aria-label="@@profiling.dotnet.list">
        @for (p of store.visible(); track p.pid) {
          <li class="proc" [cdkOption]="p.pid" [attr.data-pid]="p.pid">
            <span class="name">{{ p.name }}</span>
            <span class="mono muted">{{ p.pid }}</span>
            <span class="tag">{{ p.runtimeVersion ?? (p.runtime === 'core' ? '.NET' : '.NET Framework') }}</span>
            @if (p.arch) { <span class="tag">{{ p.arch }}</span> }
            @if (p.needsElevation) { <span class="tag warn" i18n="@@profiling.dotnet.needsAdmin">needs administrator</span> }
            @if (p.commandLine) { <span class="cmd mono muted ellipsis" [title]="p.commandLine">{{ p.commandLine }}</span> }
          </li>
        }
      </ul>
    }

    <fieldset class="kinds">
      <legend i18n="@@profiling.dotnet.kind.legend">What to record</legend>
      @for (c of kinds; track c.kind) {
        <label class="kind">
          <input type="radio" name="dotnet-kind" [value]="c.kind" [checked]="store.kind() === c.kind" (change)="store.setKind(c.kind)" />
          <span>
            <strong>{{ c.label }}</strong>
            <span class="help">{{ c.help }}</span>
          </span>
        </label>
      }
    </fieldset>

    <div class="row">
      <label class="check">
        <input type="checkbox" [checked]="store.durationSec() !== null" [disabled]="store.kind() === 'heap'" (change)="toggleDuration($event)" />
        <span i18n="@@profiling.dotnet.stopAfter">Stop after</span>
      </label>
      <label class="sr-only" for="dotnet-duration" i18n="@@profiling.dotnet.duration.label">Seconds</label>
      <input id="dotnet-duration" class="input num" type="number" min="1" max="3600" [value]="store.durationSec() ?? 30" [disabled]="store.durationSec() === null || store.kind() === 'heap'" (change)="store.setDuration(asNumber($event))" />
      <span class="muted" i18n="@@profiling.dotnet.seconds">seconds</span>
      <span class="spacer"></span>
      <button type="button" class="btn primary" (click)="store.start()" [disabled]="!store.canRecord()" data-testid="dotnet-record" i18n="@@profiling.dotnet.record">Record</button>
    </div>
    @if (store.selected()?.needsElevation) {
      <imt-tool-error [message]="elevationText" (dismissed)="store.dismissError()" />
    }
    <imt-tool-error [message]="store.error()" [code]="store.errorCode()" (dismissed)="store.dismissError()" />

    @if (store.captures().length) {
      <h3 class="section-title" i18n="@@profiling.dotnet.captures">Captures</h3>
      <ul class="caps" aria-live="polite">
        @for (c of store.captures(); track c.id) {
          <li class="cap" [attr.data-state]="c.state">
            <div class="head">
              <strong>{{ c.processName }} ({{ c.pid }})</strong>
              <span class="tag">{{ kindLabel(c.kind) }}</span>
              <span>{{ stateLabel(c.state) }}</span>
              <span class="mono muted">{{ elapsed(c) }} · {{ size(c) }}</span>
              <span class="spacer"></span>
              @if (c.state === 'recording') {
                <button type="button" class="btn" (click)="store.stop(c.id)" [attr.aria-label]="stopLabel(c)" i18n="@@profiling.dotnet.stop">Stop</button>
                <button type="button" class="btn ghost" (click)="store.cancel(c.id)" [attr.aria-label]="cancelLabel(c)" i18n="@@profiling.dotnet.cancel">Cancel</button>
              }
            </div>
            @if (c.error) { <imt-tool-error [message]="c.error" /> }
            @for (n of c.notes; track $index) { <p class="note muted">{{ n }}</p> }
            @if (c.state === 'done' && c.path) { <p class="note muted mono ellipsis" [title]="c.path">{{ c.path }}</p> }
          </li>
        }
      </ul>
    }
    <div class="foot">
      <span class="muted" i18n="@@profiling.dotnet.footnote">Nothing is loaded into the process. Recordings continue in Jobs when this dialog is closed.</span>
      <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@profiling.dotnet.close">Close</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; font-size: var(--fs-md); }
    .bar, .row, .head, .foot { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
    .bar .input { flex: 1; }
    .procs, .caps { list-style: none; margin: 0; padding: 0; max-height: 220px; overflow: auto; border: 1px solid var(--border-2); border-radius: 6px; }
    .proc { display: flex; align-items: center; gap: 8px; padding: 6px 8px; cursor: pointer; flex-wrap: wrap; }
    .proc[aria-selected='true'] { background: var(--bg-sel); }
    .proc:focus-visible { outline: 2px solid var(--accent); }
    .name { font-weight: 600; color: var(--text); }
    .cmd { flex-basis: 100%; }
    .tag { border: 1px solid var(--border-2); border-radius: 10px; padding: 0 6px; font-size: var(--fs-xs); color: var(--text-2); }
    .tag.warn { border-color: var(--lvl-warn); color: var(--text); }
    .kinds { border: 1px solid var(--border-2); border-radius: 6px; padding: 6px 10px; display: flex; flex-direction: column; gap: 6px; }
    .kind { display: flex; gap: 8px; align-items: flex-start; }
    .help { display: block; color: var(--text-2); }
    .num { width: 80px; }
    .spacer { flex: 1; }
    .cap { padding: 6px 8px; border-bottom: 1px solid var(--border-2); }
    .note { margin: 2px 0 0; font-size: var(--fs-sm); }
    .foot { justify-content: space-between; }
  `,
})
export class DotnetDialog implements OnInit {
  protected readonly ref = inject(DIALOG_REF);
  protected readonly store = inject(DotnetCaptureStore);
  protected readonly kinds = KIND_CHOICES;
  protected readonly elevationText = $localize`:@@profiling.dotnet.elevation:Recording .NET Framework processes uses ETW, which needs administrator rights.`;
  protected readonly selectedValue = computed(() => {
    const pid = this.store.selectedPid();
    return pid === null ? [] : [pid];
  });

  ngOnInit(): void {
    void this.store.refresh();
  }

  protected asValue(e: Event): string {
    return (e.target as HTMLInputElement).value;
  }

  protected asNumber(e: Event): number {
    return Number((e.target as HTMLInputElement).value);
  }

  protected onSelect(values: readonly unknown[]): void {
    const pid = values[0];
    if (typeof pid === 'number') this.store.select(pid);
  }

  protected toggleDuration(e: Event): void {
    this.store.setDuration((e.target as HTMLInputElement).checked ? 30 : null);
  }

  protected kindLabel(kind: DotnetCaptureKind): string {
    return KIND_CHOICES.find((k) => k.kind === kind)?.label ?? kind;
  }

  protected stateLabel(state: DotnetCaptureStatus['state']): string {
    return STATE_LABEL[state];
  }

  protected elapsed(c: DotnetCaptureStatus): string {
    return formatNs(c.elapsedMs * 1_000_000);
  }

  protected size(c: DotnetCaptureStatus): string {
    return formatBytes(c.bytes);
  }

  protected stopLabel(c: DotnetCaptureStatus): string {
    return $localize`:@@profiling.dotnet.stopLabel:Stop recording ${c.processName}:name:`;
  }

  protected cancelLabel(c: DotnetCaptureStatus): string {
    return $localize`:@@profiling.dotnet.cancelLabel:Cancel recording ${c.processName}:name: and delete it`;
  }

  /** Whether any capture still runs (tests). */
  readonly running = computed(() => this.store.captures().some(isRunning));
}
