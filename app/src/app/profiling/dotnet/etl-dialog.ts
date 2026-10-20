import { CdkListbox, CdkOption } from '@angular/cdk/listbox';
import { ChangeDetectionStrategy, Component, computed, inject, input, OnInit, signal } from '@angular/core';
import { EtlProcess } from '../../core/api.types';
import { baseName, errorMessage } from '../../tools/tool-errors';
import { ToolError } from '../../tools/elevation-hint';
import { DIALOG_REF } from '../../ui/dialog.service';
import { formatValue } from '../profiling-format';
import { DotnetBackend } from './dotnet-backend';
import { DotnetOpener } from './dotnet-capture.store';

/**
 * "Open process from .etl…": lists the processes in an `.etl` trace (PerfView, WPR, logman) with their CPU samples,
 * .NET ones first, and converts the chosen one into a profile. Dropping an `.etl` file opens its busiest .NET
 * process directly; this dialog is for choosing another.
 */
@Component({
  selector: 'imt-etl-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CdkListbox, CdkOption, ToolError],
  template: `
    <p class="muted mono ellipsis" [title]="path()">{{ fileName() }}</p>
    @if (loading()) {
      <p class="muted" i18n="@@profiling.etl.reading">Reading the trace…</p>
    } @else if (processes().length) {
      <ul class="procs" cdkListbox [cdkListboxValue]="selectedValue()" (cdkListboxValueChange)="onSelect($event.value)" aria-label="Processes in the trace" i18n-aria-label="@@profiling.etl.list">
        @for (p of processes(); track p.pid) {
          <li class="proc" [cdkOption]="p.pid" [attr.data-pid]="p.pid">
            <span class="name">{{ p.name }}</span>
            <span class="mono muted">{{ p.pid }}</span>
            <span class="tag">{{ samples(p) }}</span>
            @if (p.clrEvents > 0) { <span class="tag">.NET</span> }
            <span class="tag">{{ p.pointerSize === 4 ? '32-bit' : '64-bit' }}</span>
          </li>
        }
      </ul>
    }
    <imt-tool-error [message]="error()" [retry]="true" (retried)="load()" (dismissed)="error.set(null)" />
    <div class="foot">
      <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@profiling.etl.close">Close</button>
      <button type="button" class="btn primary" [disabled]="selected() === null || converting()" (click)="convert()" data-testid="etl-open">{{ openLabel() }}</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; font-size: 13px; }
    .procs { list-style: none; margin: 0; padding: 0; max-height: 300px; overflow: auto; border: 1px solid var(--border-2); border-radius: 6px; }
    .proc { display: flex; align-items: center; gap: 8px; padding: 6px 8px; cursor: pointer; }
    .proc[aria-selected='true'] { background: var(--bg-sel); }
    .proc:focus-visible { outline: 2px solid var(--accent); }
    .name { font-weight: 600; }
    .tag { border: 1px solid var(--border-2); border-radius: 10px; padding: 0 6px; font-size: 11px; color: var(--text-2); }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class EtlDialog implements OnInit {
  protected readonly ref = inject(DIALOG_REF);
  private readonly backend = inject(DotnetBackend);
  private readonly opener = inject(DotnetOpener);
  readonly path = input.required<string>();
  protected readonly processes = signal<EtlProcess[]>([]);
  protected readonly selected = signal<number | null>(null);
  protected readonly loading = signal(false);
  protected readonly converting = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly fileName = computed(() => baseName(this.path()));
  protected readonly selectedValue = computed(() => {
    const pid = this.selected();
    return pid === null ? [] : [pid];
  });
  protected readonly openLabel = computed(() =>
    this.converting() ? $localize`:@@profiling.etl.converting:Converting…` : $localize`:@@profiling.etl.open:Open process`,
  );

  ngOnInit(): void {
    void this.load();
  }

  async load(): Promise<void> {
    this.loading.set(true);
    this.error.set(null);
    try {
      const list = await this.backend.etlProcesses(this.path());
      this.processes.set(list);
      this.selected.set(list[0]?.pid ?? null);
    } catch (e) {
      this.error.set(errorMessage(e));
    } finally {
      this.loading.set(false);
    }
  }

  protected onSelect(values: readonly unknown[]): void {
    const pid = values[0];
    if (typeof pid === 'number') this.selected.set(pid);
  }

  protected samples(p: EtlProcess): string {
    return $localize`:@@profiling.etl.samples:${formatValue(p.samples, 'count')}:count: samples`;
  }

  async convert(): Promise<void> {
    const pid = this.selected();
    if (pid === null || this.converting()) return;
    this.converting.set(true);
    this.error.set(null);
    try {
      const r = await this.backend.etlConvert(this.path(), pid);
      await this.opener.open(r.path);
      this.ref.close();
    } catch (e) {
      this.error.set(errorMessage(e));
    } finally {
      this.converting.set(false);
    }
  }
}
