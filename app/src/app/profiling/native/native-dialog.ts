import { CdkListbox, CdkOption } from '@angular/cdk/listbox';
import { ChangeDetectionStrategy, Component, computed, inject, OnInit } from '@angular/core';
import { NativeCaptureMode, NativeCaptureStatus } from '../../core/api.types';
import { ToolError } from '../../tools/elevation-hint';
import { DIALOG_REF } from '../../ui/dialog.service';
import { formatBytes, formatNs, formatValue } from '../profiling-format';
import { MAX_RATE_HZ, MIN_RATE_HZ, NativeCaptureStore } from './native-capture.store';

interface ModeChoice {
  mode: NativeCaptureMode;
  label: string;
  help: string;
}

/** How each mode records, what it costs and what it needs (plain words for the dialog). */
export const MODE_CHOICES: ModeChoice[] = [
  {
    mode: 'sampling',
    label: $localize`:@@profiling.native.mode.sampling:Sampling (no administrator rights)`,
    help: $localize`:@@profiling.native.mode.sampling.help:IMT pauses one thread of the process at a time for a few microseconds, copies its stack and lets it run again, about 100 times a second per thread. Shows where threads spend CPU time and where they wait (wall clock, approximate). Works for your own processes.`,
  },
  {
    mode: 'etw',
    label: $localize`:@@profiling.native.mode.etw:ETW (administrator)`,
    help: $localize`:@@profiling.native.mode.etw.help:Windows records every CPU sample (1000 per second), every wait with the thread that ended it and, if chosen, heap allocations with their call stacks. Works for any process. Asks for administrator rights the first time: the recorder runs in a separate elevated process.`,
  },
];

const STATE_LABEL: Record<NativeCaptureStatus['state'], string> = {
  recording: $localize`:@@profiling.native.state.recording:Recording`,
  stopping: $localize`:@@profiling.native.state.stopping:Finishing (resolving symbols)…`,
  done: $localize`:@@profiling.native.state.done:Done`,
  failed: $localize`:@@profiling.native.state.failed:Failed`,
  cancelled: $localize`:@@profiling.native.state.cancelled:Cancelled`,
};

/**
 * "Profile native process…": pick a process, choose how to record, then watch it record. Nothing is loaded into
 * the process. A finished capture opens on its findings; captures also run as Jobs, so the dialog may be closed.
 */
@Component({
  selector: 'imt-native-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CdkListbox, CdkOption, ToolError],
  template: `
    <div class="bar">
      <label class="sr-only" for="native-filter" i18n="@@profiling.native.filter.label">Filter processes</label>
      <input id="native-filter" class="input" type="search" maxlength="200" placeholder="Filter by name, id or path" i18n-placeholder="@@profiling.native.filter.placeholder" [value]="store.filter()" (input)="store.setFilter(asValue($event))" />
      <button type="button" class="btn" (click)="store.refresh()" [disabled]="store.loading()" i18n="@@profiling.native.refresh">Refresh</button>
    </div>
    @if (store.loading() && !store.processes().length) {
      <p class="muted" i18n="@@profiling.native.loading">Listing processes…</p>
    } @else if (!store.visible().length) {
      <p class="muted" i18n="@@profiling.native.none">No process matches. Start the application, then Refresh.</p>
    } @else {
      <ul class="procs" cdkListbox [cdkListboxValue]="selectedValue()" (cdkListboxValueChange)="onSelect($event.value)" aria-label="Processes" i18n-aria-label="@@profiling.native.list">
        @for (p of store.visible(); track p.pid) {
          <li class="proc" [cdkOption]="p.pid" [attr.data-pid]="p.pid">
            <span class="name">{{ p.name }}</span>
            <span class="mono muted">{{ p.pid }}</span>
            @if (p.arch) { <span class="tag">{{ p.arch }}</span> }
            @if (!p.accessible) { <span class="tag warn" i18n="@@profiling.native.etwOnly">ETW only</span> }
            @if (p.path) { <span class="cmd mono muted ellipsis" [title]="p.path">{{ p.path }}</span> }
          </li>
        }
      </ul>
    }

    <fieldset class="kinds">
      <legend i18n="@@profiling.native.mode.legend">How to record</legend>
      @for (c of modes; track c.mode) {
        <label class="kind">
          <input type="radio" name="native-mode" [value]="c.mode" [checked]="store.mode() === c.mode" (change)="store.setMode(c.mode)" />
          <span>
            <strong>{{ c.label }}</strong>
            <span class="help">{{ c.help }}</span>
          </span>
        </label>
      }
    </fieldset>

    <div class="row">
      @if (store.mode() === 'sampling') {
        <label for="native-rate" i18n="@@profiling.native.rate">Samples per second per thread</label>
        <input id="native-rate" class="input num" type="number" [min]="minRate" [max]="maxRate" [value]="store.rateHz()" (change)="store.setRate(asNumber($event))" />
      } @else {
        <label class="check">
          <input type="checkbox" [checked]="store.heap()" (change)="store.setHeap(asChecked($event))" />
          <span i18n="@@profiling.native.heap">Heap allocations with their stacks (slows allocation-heavy processes)</span>
        </label>
      }
    </div>
    <div class="row">
      <label class="check">
        <input type="checkbox" [checked]="store.durationSec() !== null" (change)="toggleDuration($event)" />
        <span i18n="@@profiling.native.stopAfter">Stop after</span>
      </label>
      <label class="sr-only" for="native-duration" i18n="@@profiling.native.duration.label">Seconds</label>
      <input id="native-duration" class="input num" type="number" min="1" max="3600" [value]="store.durationSec() ?? 30" [disabled]="store.durationSec() === null" (change)="store.setDuration(asNumber($event))" />
      <span class="muted" i18n="@@profiling.native.seconds">seconds</span>
    </div>
    <fieldset class="kinds">
      <legend i18n="@@profiling.native.symbols">Symbols</legend>
      <label for="native-symbols" i18n="@@profiling.native.symbolPath">Symbol path (folders with PDB files, separated by ;)</label>
      <input id="native-symbols" class="input mono" type="text" maxlength="4096" placeholder="C:\\symbols;D:\\build\\pdb" [value]="store.symbolPath()" (change)="store.setSymbolPath(asValue($event))" />
      <label class="check">
        <input type="checkbox" [checked]="store.symbolServer()" (change)="store.setSymbolServer(asChecked($event))" />
        <span i18n="@@profiling.native.symbolServer">Download missing PDBs of Windows modules from the Microsoft symbol server</span>
      </label>
    </fieldset>

    <div class="row">
      <span class="spacer"></span>
      <button type="button" class="btn primary" (click)="store.start()" [disabled]="!store.canRecord()" data-testid="native-record" i18n="@@profiling.native.record">Record</button>
    </div>
    @if (store.needsEtw()) {
      <imt-tool-error [message]="etwOnlyText" />
    }
    <imt-tool-error [message]="store.error()" [code]="store.errorCode()" (dismissed)="store.dismissError()" />

    @if (store.captures().length) {
      <h3 class="section-title" i18n="@@profiling.native.captures">Captures</h3>
      <ul class="caps" aria-live="polite">
        @for (c of store.captures(); track c.id) {
          <li class="cap" [attr.data-state]="c.state">
            <div class="head">
              <strong>{{ c.processName }} ({{ c.pid }})</strong>
              <span class="tag">{{ modeLabel(c.mode) }}</span>
              <span>{{ stateLabel(c.state) }}</span>
              <span class="mono muted">{{ progress(c) }}</span>
              <span class="spacer"></span>
              @if (c.state === 'recording') {
                <button type="button" class="btn" (click)="store.stop(c)" [attr.aria-label]="stopLabel(c)" i18n="@@profiling.native.stop">Stop</button>
                <button type="button" class="btn ghost" (click)="store.cancel(c)" [attr.aria-label]="cancelLabel(c)" i18n="@@profiling.native.cancel">Cancel</button>
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
      <span class="muted" i18n="@@profiling.native.footnote">Nothing is loaded into the process. Recordings continue in Jobs when this dialog is closed.</span>
      <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@profiling.native.close">Close</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; font-size: var(--fs-md); }
    .bar, .row, .head, .foot { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
    .bar .input { flex: 1; }
    .procs, .caps { list-style: none; margin: 0; padding: 0; max-height: 200px; overflow: auto; border: 1px solid var(--border-2); border-radius: 6px; }
    .proc { display: flex; align-items: center; gap: 8px; padding: 6px 8px; cursor: pointer; flex-wrap: wrap; }
    .proc[aria-selected='true'] { background: var(--bg-sel); }
    .proc:focus-visible { outline: 2px solid var(--accent); }
    .name { font-weight: 600; color: var(--text); }
    .cmd { flex-basis: 100%; }
    .tag { border: 1px solid var(--border-2); border-radius: 10px; padding: 0 6px; font-size: var(--fs-xs); color: var(--text-2); }
    .tag.warn { border-color: var(--lvl-warn); color: var(--text); }
    .kinds { border: 1px solid var(--border-2); border-radius: 6px; padding: 6px 10px; display: flex; flex-direction: column; gap: 6px; }
    .kind, .check { display: flex; gap: 8px; align-items: flex-start; }
    .help { display: block; color: var(--text-2); }
    .num { width: 80px; }
    .spacer { flex: 1; }
    .cap { padding: 6px 8px; border-bottom: 1px solid var(--border-2); }
    .note { margin: 2px 0 0; font-size: var(--fs-sm); }
    .foot { justify-content: space-between; }
  `,
})
export class NativeDialog implements OnInit {
  protected readonly ref = inject(DIALOG_REF);
  protected readonly store = inject(NativeCaptureStore);
  protected readonly modes = MODE_CHOICES;
  protected readonly minRate = MIN_RATE_HZ;
  protected readonly maxRate = MAX_RATE_HZ;
  protected readonly etwOnlyText = $localize`:@@profiling.native.etwOnlyText:IMT cannot read this process without administrator rights (it runs elevated or as another user). Choose ETW (administrator) to record it.`;
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

  protected asChecked(e: Event): boolean {
    return (e.target as HTMLInputElement).checked;
  }

  protected onSelect(values: readonly unknown[]): void {
    const pid = values[0];
    if (typeof pid === 'number') this.store.select(pid);
  }

  protected toggleDuration(e: Event): void {
    this.store.setDuration(this.asChecked(e) ? 30 : null);
  }

  protected modeLabel(mode: NativeCaptureMode): string {
    return mode === 'etw' ? 'ETW' : $localize`:@@profiling.native.tag.sampling:Sampling`;
  }

  protected stateLabel(state: NativeCaptureStatus['state']): string {
    return STATE_LABEL[state];
  }

  protected progress(c: NativeCaptureStatus): string {
    const parts = [formatNs(c.elapsedMs * 1_000_000)];
    if (c.mode === 'sampling') parts.push($localize`:@@profiling.native.samples:${formatValue(c.samples, 'count')}:count: samples`);
    else parts.push(formatBytes(c.bytes));
    return parts.join(' · ');
  }

  protected stopLabel(c: NativeCaptureStatus): string {
    return $localize`:@@profiling.native.stopLabel:Stop recording ${c.processName}:name:`;
  }

  protected cancelLabel(c: NativeCaptureStatus): string {
    return $localize`:@@profiling.native.cancelLabel:Cancel recording ${c.processName}:name: and delete it`;
  }
}
