import { ChangeDetectionStrategy, Component, computed, inject, OnInit, signal } from '@angular/core';
import { Icon } from '../../shared/icon';
import { HelpHint } from '../../ui/help-hint';
import { LogFormat } from '../../core/tools.types';
import { copyText } from '../clipboard';
import { ToolError } from '../elevation-hint';
import { ToolUnavailable } from '../tool-unavailable';
import { optionalNumber } from '../collector/plan-model';
import { presetState } from './perfmon-model';
import { PerfmonStore } from './perfmon.store';

/** Performance Monitor wizard: choose what to watch, generate the Data Collector Set, install it, import the results. */
@Component({
  selector: 'imt-perfmon-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, HelpHint, ToolError, ToolUnavailable],
  template: `
    <div class="scroll" role="region" aria-label="Performance Monitor" i18n-aria-label="@@tools.perfmon.region" tabindex="0">
      <p class="intro">
        <ng-container i18n="@@tools.perfmon.intro">Record Windows performance counters while you reproduce a problem, then open the result here next to your logs.</ng-container>
        @if (!store.onWindows()) {
          <ng-container i18n="@@tools.perfmon.notWindows"><strong>Collector sets can only be created on Windows.</strong> Here you can still generate the XML and the commands to run on the Windows machine.</ng-container>
        }
      </p>
      <imt-tool-unavailable tool="perfmon.templates" />

      <imt-tool-error [message]="store.error()" (dismissed)="store.dismissError()" />
      @if (store.notice(); as notice) { <div class="note" role="status">{{ notice }}</div> }

      <section aria-labelledby="pm-what">
        <h2 id="pm-what" class="section-title" i18n="@@tools.perfmon.step1">1 · What to watch</h2>
        <label class="lbl"><ng-container i18n="@@tools.perfmon.collectorName.label">Collector name</ng-container>
          <input class="input" aria-label="Collector name" i18n-aria-label="@@tools.perfmon.collectorName.aria" [value]="store.draft().name" (input)="store.patchDraft({ name: val($event) })" />
        </label>
        <div class="lbl">
          <span i18n="@@tools.perfmon.processes">Processes <imt-help label="Processes" i18n-label="@@tools.perfmon.processes.helpLabel" text="Instance names as Performance Monitor shows them, for example w3wp or OrderService#1. Leave empty to watch every process." i18n-text="@@tools.perfmon.processes.helpText" /></span>
          <div class="chips">
            @for (p of store.draft().processes; track p) {
              <span class="chip">{{ p }}<button type="button" class="x" [attr.aria-label]="removeProcessLabel(p)" (click)="store.removeProcess(p)"><imt-icon name="close" [size]="10" /></button></span>
            } @empty { <span class="faint" i18n="@@tools.perfmon.allProcesses">All processes</span> }
          </div>
          <div class="inline">
            <input #proc class="input" aria-label="Process name" i18n-aria-label="@@tools.perfmon.processName.aria" placeholder="e.g. OrderService" i18n-placeholder="@@tools.perfmon.processName.placeholder" (keydown.enter)="addProcess(proc)" />
            <button type="button" class="btn" (click)="addProcess(proc)" i18n="@@tools.perfmon.addProcess">Add process</button>
          </div>
        </div>
      </section>

      <section aria-labelledby="pm-counters">
        <h2 id="pm-counters" class="section-title" i18n="@@tools.perfmon.step2">2 · Counters</h2>
        @for (preset of store.presets(); track preset.id) {
          <div class="preset">
            <div class="head">
              <button type="button" class="icon-btn" [attr.aria-expanded]="isOpen(preset.id)" [attr.aria-label]="toggleCountersLabel(preset.id, preset.name)" (click)="toggleOpen(preset.id)">
                <imt-icon [name]="isOpen(preset.id) ? 'chevronDown' : 'chevronRight'" [size]="14" />
              </button>
              <label class="check">
                <input type="checkbox" [checked]="state(preset) === 'all'" [indeterminate]="state(preset) === 'some'" [attr.aria-label]="presetLabel(preset.name)" (change)="store.togglePreset(preset)" />
                <strong>{{ preset.name }}</strong>
              </label>
              <span class="faint">{{ preset.description }}</span>
            </div>
            @if (isOpen(preset.id)) {
              <ul class="counters">
                @for (c of preset.counters; track c.path) {
                  <li>
                    <label class="check">
                      <input type="checkbox" [checked]="store.draft().counters.includes(c.path)" (change)="store.toggleCounter(c.path)" />
                      <span class="mono path">{{ c.path }}</span>
                    </label>
                    @if (c.guidance) { <div class="guide" i18n="@@tools.perfmon.guidance"><span class="faint">What to look for:</span> {{ c.guidance }}</div> }
                  </li>
                }
              </ul>
            }
          </div>
        }
      </section>

      <section aria-labelledby="pm-out">
        <h2 id="pm-out" class="section-title" i18n="@@tools.perfmon.step3">3 · Sampling and output</h2>
        <div class="row">
          <label class="lbl"><ng-container i18n="@@tools.perfmon.interval">Interval (seconds)</ng-container>
            <input class="input" type="number" min="1" max="86400" [value]="store.draft().intervalSecs" (input)="store.patchDraft({ intervalSecs: +val($event) })" />
          </label>
          <label class="lbl"><ng-container i18n="@@tools.perfmon.format">Format</ng-container>
            <select class="select" [value]="store.draft().format" (change)="store.patchDraft({ format: fmt($event) })">
              <option value="csv">CSV</option>
              <option value="tsv">TSV</option>
              <option value="blg" i18n="@@tools.perfmon.format.blg">BLG (binary)</option>
            </select>
          </label>
          <label class="lbl"><ng-container i18n="@@tools.perfmon.maxSize">Maximum size (MB, circular)</ng-container>
            <input class="input" type="number" min="1" [disabled]="store.draft().format !== 'blg'" [value]="store.draft().maxSizeMb ?? ''" placeholder="BLG only" i18n-placeholder="@@tools.perfmon.maxSize.placeholder" (input)="store.patchDraft({ maxSizeMb: num($event) })" />
          </label>
          <label class="lbl"><ng-container i18n="@@tools.perfmon.duration">Duration (minutes)</ng-container>
            <input class="input" type="number" min="1" [value]="store.draft().durationMinutes ?? ''" placeholder="until stopped" i18n-placeholder="@@tools.perfmon.duration.placeholder" (input)="store.patchDraft({ durationMinutes: num($event) })" />
          </label>
        </div>
        <label class="lbl"><ng-container i18n="@@tools.perfmon.outputFolder">Output folder</ng-container>
          <input class="input mono" [value]="store.draft().outputDir" (input)="store.patchDraft({ outputDir: val($event) })" />
        </label>
      </section>

      <section aria-labelledby="pm-gen">
        <h2 id="pm-gen" class="section-title" i18n="@@tools.perfmon.step4">4 · Generate</h2>
        @for (p of store.problems(); track p) { <div class="problem">{{ p }}</div> }
        <div class="inline">
          <button type="button" class="btn primary" [disabled]="!!store.busy() || store.problems().length > 0 || !store.canGenerate()" (click)="store.generate()" i18n="@@tools.perfmon.generateXml">Generate XML</button>
          @if (store.canManage()) {
            <button type="button" class="btn" [disabled]="!!store.busy() || store.problems().length > 0" (click)="store.installAndStart()" i18n="@@tools.perfmon.installStart">Install &amp; start</button>
          }
          @if (store.busy(); as busy) { <span class="faint" role="status">{{ busy }}</span> }
        </div>
        @if (store.generated(); as g) {
          <div class="xml-head">
            <strong i18n="@@tools.perfmon.xmlTitle">Data Collector Set XML</strong>
            <span class="spacer"></span>
            <button type="button" class="btn" (click)="copy(g.xml)">{{ copyXmlLabel() }}</button>
            <button type="button" class="btn" (click)="store.saveXml()" i18n="@@tools.perfmon.saveAs">Save as…</button>
          </div>
          <pre class="xml mono" tabindex="0" aria-label="Generated XML" i18n-aria-label="@@tools.perfmon.xml.aria">{{ g.xml }}</pre>
          <strong i18n="@@tools.perfmon.commands">Commands</strong>
          <p class="faint" i18n="@@tools.perfmon.commands.hint">Run these in an elevated Command Prompt to import and control the set from the XML file.</p>
          <ul class="cmds">
            @for (cmd of g.commands; track cmd) {
              <li><code class="mono">{{ cmd }}</code><button type="button" class="icon-btn" [attr.aria-label]="copyCommandLabel(cmd)" title="Copy" i18n-title="@@tools.perfmon.copy.title" (click)="copy(cmd)"><imt-icon name="copy" [size]="13" /></button></li>
            }
          </ul>
        }
      </section>

      @if (store.status(); as s) {
        <section aria-labelledby="pm-status" class="card" data-testid="collector-status">
          <h2 id="pm-status" class="section-title" i18n="@@tools.perfmon.collectorSet">Collector set</h2>
          <div class="status-row">
            <strong>{{ s.name }}</strong>
            <span class="badge" [class.running]="s.status === 'running'" data-testid="collector-state">{{ s.status }}</span>
          </div>
          <div class="faint" i18n="@@tools.perfmon.output">Output: <span class="mono">{{ s.outputLocation || s.rootPath }}</span></div>
          <div class="inline">
            <button type="button" class="btn" [disabled]="!!store.busy()" (click)="store.control('query')" i18n="@@tools.perfmon.refresh"><imt-icon name="refresh" [size]="12" />Refresh</button>
            @if (s.status === 'running') {
              <button type="button" class="btn" [disabled]="!!store.busy()" (click)="store.control('stop')" i18n="@@tools.perfmon.btn.stop">Stop</button>
            } @else {
              <button type="button" class="btn" [disabled]="!!store.busy()" (click)="store.control('start')" i18n="@@tools.perfmon.btn.start">Start</button>
            }
            <button type="button" class="btn" [disabled]="!!store.busy()" (click)="store.control('delete')" i18n="@@tools.perfmon.btn.delete">Delete</button>
          </div>
          <div class="inline">
            <button type="button" class="btn primary" [disabled]="!!store.busy()" (click)="store.importResults()" i18n="@@tools.perfmon.importResults">Import results</button>
            <label class="check" i18n="@@tools.perfmon.liveTail"><input type="checkbox" [checked]="store.liveTail()" (change)="store.setLiveTail(checked($event))" /> Live tail</label>
          </div>
        </section>
      }
    </div>
  `,
  styles: `
    :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .scroll { flex: 1; min-height: 0; overflow: auto; padding: 14px 18px; display: flex; flex-direction: column; gap: 14px; max-width: 880px; }
    .scroll:focus-visible { outline-offset: -2px; }
    section { display: flex; flex-direction: column; gap: 8px; }
    h2 { margin: 0; }
    .intro { margin: 0; color: var(--text-2); line-height: 1.5; }
    .lbl { display: flex; flex-direction: column; gap: 4px; font-size: 11px; color: var(--muted); }
    .row { display: flex; gap: 12px; flex-wrap: wrap; }
    .row .lbl { flex: 1; min-width: 140px; }
    .inline { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
    .chips { display: flex; gap: 6px; flex-wrap: wrap; align-items: center; }
    .chip .x { border: 0; background: transparent; color: var(--muted); cursor: pointer; padding: 0; display: inline-flex; }
    .preset { border: 1px solid var(--border-2); border-radius: 8px; background: var(--bg-2); padding: 6px 10px; }
    .preset .head { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
    .check { display: inline-flex; gap: 6px; align-items: center; color: var(--text-2); }
    .counters { list-style: none; margin: 8px 0 2px; padding: 0; display: flex; flex-direction: column; gap: 6px; }
    .path { font-size: 11.5px; }
    .guide { margin-left: 24px; font-size: 12px; color: var(--text-2); }
    .problem { color: var(--danger-text); font-size: 12px; }
    .note { padding: 6px 10px; border-radius: 6px; border: 1px solid var(--ok); background: var(--bg-3); font-size: 12px; }
    .xml-head { display: flex; gap: 8px; align-items: center; margin-top: 6px; }
    .spacer { flex: 1; }
    .xml { margin: 0; padding: 8px 10px; max-height: 260px; overflow: auto; border-radius: 6px; background: var(--code-bg); border: 1px solid var(--border); font-size: 11.5px; line-height: 1.5; }
    .cmds { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
    .cmds li { display: flex; align-items: center; gap: 6px; }
    .cmds code { flex: 1; min-width: 0; overflow-x: auto; white-space: nowrap; padding: 3px 8px; border-radius: 5px; background: var(--code-bg); font-size: 11.5px; }
    .card { padding: 10px 12px; border: 1px solid var(--border-3); border-radius: 8px; background: var(--bg-2); }
    .status-row { display: flex; gap: 10px; align-items: center; }
    .badge.running { background: var(--accent-soft); color: var(--chip-text); }
  `,
})
export class PerfmonView implements OnInit {
  protected readonly store = inject(PerfmonStore);
  protected readonly copied = signal<string | null>(null);
  private readonly expanded = signal<ReadonlySet<string>>(new Set());
  protected readonly copyXmlLabel = computed(() =>
    this.copied() === 'xml' ? $localize`:@@tools.perfmon.copied:Copied` : $localize`:@@tools.perfmon.copyXml:Copy XML`,
  );

  protected removeProcessLabel = (p: string) => $localize`:@@tools.perfmon.removeProcess:Remove process ${p}:name:`;
  protected presetLabel = (name: string) => $localize`:@@tools.perfmon.preset:Preset ${name}:name:`;
  protected copyCommandLabel = (cmd: string) => $localize`:@@tools.perfmon.copyCommand:Copy command ${cmd}:command:`;
  protected toggleCountersLabel(id: string, name: string): string {
    return this.isOpen(id)
      ? $localize`:@@tools.perfmon.hideCounters:Hide counters of ${name}:name:`
      : $localize`:@@tools.perfmon.showCounters:Show counters of ${name}:name:`;
  }

  ngOnInit(): void {
    void this.store.loadPresets();
  }

  protected isOpen = (id: string) => this.expanded().has(id);
  protected toggleOpen(id: string): void {
    this.expanded.update((s) => {
      const next = new Set(s);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  }
  protected state = (preset: Parameters<typeof presetState>[0]) => presetState(preset, this.store.draft().counters);
  protected val = (e: Event) => (e.target as HTMLInputElement).value;
  protected checked = (e: Event) => (e.target as HTMLInputElement).checked;
  protected num = (e: Event) => optionalNumber(this.val(e));
  protected fmt = (e: Event) => this.val(e) as LogFormat;

  protected addProcess(input: HTMLInputElement): void {
    this.store.addProcess(input.value);
    input.value = '';
    input.focus();
  }

  protected async copy(text: string): Promise<void> {
    this.copied.set((await copyText(text)) ? (text.startsWith('<?xml') ? 'xml' : 'cmd') : null);
  }
}
