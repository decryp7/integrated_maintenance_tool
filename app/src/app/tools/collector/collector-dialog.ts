import { ChangeDetectionStrategy, Component, computed, inject, OnInit, signal } from '@angular/core';
import { Platform } from '../../core/platform';
import { CollectBackend } from '../../core/tools-backend';
import { CollectPlan, CollectResult, CustomPattern, LogSpec } from '../../core/tools.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { Icon } from '../../shared/icon';
import { DIALOG_REF } from '../../ui/dialog.service';
import { HelpHint } from '../../ui/help-hint';
import { OverwriteGuard } from '../../ui/overwrite-guard';
import { ToolError } from '../elevation-hint';
import { TOOL_IDS } from '../../core/tools.types';
import { ToolCatalog } from '../tool-catalog.store';
import { ToolUnavailable } from '../tool-unavailable';
import { errorMessage } from '../tool-errors';
import { formatBytes, formatCount } from '../../shared/format/units';
import { DEFAULT_PLAN, joinLines, ManifestSummary, newLogSpec, optionalNumber, parsePlan, splitLines, stringifyPlan, summarize } from './plan-model';

type Phase = 'idle' | 'running' | 'done';

/** Form over the collection plan, with an "Edit as JSON" escape hatch, a progress state and the manifest summary. */
@Component({
  selector: 'debrief-collector-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, HelpHint, ToolError, ToolUnavailable],
  template: `
    <p class="intro" i18n="@@tools.collector.intro">Gather logs, system information and counters into one bundle (.debriefz) you can open here or send to support. Secrets are masked before anything is written.</p>
    <debrief-tool-unavailable tool="diagnostics" />
    <debrief-tool-error [message]="error()" (dismissed)="error.set(null)" />

    <div class="bar">
      <button type="button" class="btn" [class.active]="jsonMode()" [attr.aria-pressed]="jsonMode()" [disabled]="phase() === 'running'" (click)="toggleJson()" i18n="@@tools.collector.editAsJson">Edit as JSON</button>
    </div>

    @if (jsonMode()) {
      <label class="lbl"><ng-container i18n="@@tools.collector.planJson.label">Plan (JSON)</ng-container>
        <textarea class="input mono json" rows="16" aria-label="Plan JSON" i18n-aria-label="@@tools.collector.planJson.aria" [value]="jsonText()" (input)="onJson($event)"></textarea>
      </label>
      @if (jsonError(); as je) { <div class="err" role="alert">{{ je }}</div> }
    } @else {
      <fieldset [disabled]="phase() === 'running'">
        <label class="lbl"><ng-container i18n="@@tools.collector.bundleName.label">Bundle name</ng-container>
          <input class="input" aria-label="Bundle name" i18n-aria-label="@@tools.collector.bundleName.aria" [value]="plan().name" (input)="patch({ name: val($event) })" />
        </label>

        <legend class="section-title" i18n="@@tools.collector.logFolders">Log folders</legend>
        @for (l of plan().logs; track $index) {
          <div class="log">
            <div class="inline">
              <input class="input grow mono" [attr.aria-label]="logFolderLabel($index)" placeholder="C:\\ProgramData\\Contoso\\Logs" [value]="l.path" (input)="setLog($index, { path: val($event) })" />
              <button type="button" class="icon-btn" [attr.aria-label]="removeLogLabel($index)" title="Remove" i18n-title="@@tools.collector.remove.title" (click)="removeLog($index)"><debrief-icon name="close" [size]="13" /></button>
            </div>
            <div class="row">
              <label class="lbl"><ng-container i18n="@@tools.collector.include.label">Include (globs, comma separated)</ng-container>
                <input class="input mono" [attr.aria-label]="includeLabel($index)" [value]="l.include.join(', ')" (input)="setLog($index, { include: csv($event) })" />
              </label>
              <label class="lbl"><ng-container i18n="@@tools.collector.maxAge.label">Max age (days)</ng-container>
                <input class="input" type="number" min="0" [attr.aria-label]="maxAgeLabel($index)" [value]="l.maxAgeDays ?? ''" placeholder="any" i18n-placeholder="@@tools.collector.maxAge.placeholder" (input)="setLog($index, { maxAgeDays: num($event) })" />
              </label>
              <label class="lbl"><ng-container i18n="@@tools.collector.maxSize.label">Max size (MB)</ng-container>
                <input class="input" type="number" min="0" [attr.aria-label]="maxSizeLabel($index)" [value]="l.maxTotalMb ?? ''" placeholder="no limit" i18n-placeholder="@@tools.collector.maxSize.placeholder" (input)="setLog($index, { maxTotalMb: num($event) })" />
              </label>
            </div>
          </div>
        }
        <div><button type="button" class="btn" (click)="addLog()"><debrief-icon name="plus" [size]="12" /><ng-container i18n="@@tools.collector.addLogFolder">Add log folder</ng-container></button></div>

        <legend class="section-title" i18n="@@tools.collector.systemInfo">System information</legend>
        <div class="checks">
          <label class="check" i18n="@@tools.collector.check.environment"><input type="checkbox" [checked]="plan().environment" (change)="patch({ environment: checked($event) })" /> Environment (OS, CPU, memory, disks)</label>
          <label class="check" i18n="@@tools.collector.check.msinfo"><input type="checkbox" [checked]="plan().msinfo" (change)="patch({ msinfo: checked($event) })" /> System information (msinfo32, Windows)</label>
          <label class="check" i18n="@@tools.collector.check.counters"><input type="checkbox" [checked]="plan().counters" (change)="patch({ counters: checked($event) })" /> Performance counter sample (Windows)</label>
          <label class="check" i18n="@@tools.collector.check.macosLog"><input type="checkbox" [checked]="plan().macosUnifiedLog" (change)="patch({ macosUnifiedLog: checked($event) })" /> macOS unified log</label>
          <label class="check" i18n="@@tools.collector.check.linuxJournal"><input type="checkbox" [checked]="plan().linuxJournal" (change)="patch({ linuxJournal: checked($event) })" /> Linux journal</label>
        </div>
        <div class="row">
          <label class="lbl"><ng-container i18n="@@tools.collector.eventLogs.label">Event logs (Windows, one per line)</ng-container>
            <textarea class="input mono" rows="3" aria-label="Event logs" i18n-aria-label="@@tools.collector.eventLogs.aria" [value]="lines(plan().eventLogs)" (input)="patch({ eventLogs: split($event) })"></textarea>
          </label>
          <label class="lbl"><ng-container i18n="@@tools.collector.registry.label">Registry keys (Windows, one per line)</ng-container>
            <textarea class="input mono" rows="3" aria-label="Registry keys" i18n-aria-label="@@tools.collector.registry.aria" placeholder="HKLM\\SOFTWARE\\Contoso" [value]="lines(plan().registry)" (input)="patch({ registry: split($event) })"></textarea>
          </label>
        </div>
        <div class="row">
          <label class="lbl"><ng-container i18n="@@tools.collector.eventLogDays">Event log days</ng-container>
            <input class="input" type="number" min="1" [value]="plan().eventLogDays" (input)="patch({ eventLogDays: +val($event) })" />
          </label>
          <label class="lbl"><ng-container i18n="@@tools.collector.recentLogHours">Recent system log hours</ng-container>
            <input class="input" type="number" min="1" [value]="plan().recentLogHours" (input)="patch({ recentLogHours: +val($event) })" />
          </label>
        </div>
        @if (plan().counters) {
          <label class="lbl"><ng-container i18n="@@tools.collector.counters.label">Counters (one per line)</ng-container>
            <textarea class="input mono" rows="3" aria-label="Counter paths" i18n-aria-label="@@tools.collector.counters.aria" [value]="lines(plan().counterPaths)" (input)="patch({ counterPaths: split($event) })"></textarea>
          </label>
        }
        @if (plan().environment) {
          <label class="lbl"><ng-container i18n="@@tools.collector.envVars.label">Environment variables to include (one per line)</ng-container>
            <textarea class="input mono" rows="3" aria-label="Environment variables" i18n-aria-label="@@tools.collector.envVars.aria" [value]="lines(plan().envAllowlist)" (input)="patch({ envAllowlist: split($event) })"></textarea>
          </label>
        }

        <legend class="section-title" i18n="@@tools.collector.redaction">Redaction <debrief-help label="Redaction" i18n-label="@@tools.collector.redaction.helpLabel" text="Text files are scanned and matches are replaced before they enter the bundle. The manifest lists how many values each rule masked." i18n-text="@@tools.collector.redaction.helpText" /></legend>
        <div class="checks">
          <label class="check" i18n="@@tools.collector.mask.builtIn"><input type="checkbox" [checked]="plan().redact.builtIn" (change)="patchRedact({ builtIn: checked($event) })" /> Mask e-mail addresses, tokens, passwords and card numbers</label>
          <label class="check" i18n="@@tools.collector.mask.ip"><input type="checkbox" [checked]="plan().redact.ipAddresses" (change)="patchRedact({ ipAddresses: checked($event) })" /> Mask IP addresses</label>
        </div>
        @for (p of plan().redact.patterns; track $index) {
          <div class="inline">
            <input class="input" [attr.aria-label]="patternNameLabel($index)" placeholder="name" i18n-placeholder="@@tools.collector.pattern.namePlaceholder" [value]="p.name" (input)="setPattern($index, { name: val($event) })" />
            <input class="input grow mono" [attr.aria-label]="patternRegexLabel($index)" placeholder="regular expression" i18n-placeholder="@@tools.collector.pattern.regexPlaceholder" [value]="p.regex" (input)="setPattern($index, { regex: val($event) })" />
            <input class="input" [attr.aria-label]="patternReplaceLabel($index)" placeholder="[REDACTED]" [value]="p.replace" (input)="setPattern($index, { replace: val($event) })" />
            <button type="button" class="icon-btn" [attr.aria-label]="removePatternLabel($index)" title="Remove" i18n-title="@@tools.collector.remove.title" (click)="removePattern($index)"><debrief-icon name="close" [size]="13" /></button>
          </div>
        }
        <div><button type="button" class="btn" (click)="addPattern()"><debrief-icon name="plus" [size]="12" /><ng-container i18n="@@tools.collector.addPattern">Add custom pattern</ng-container></button></div>

        <legend class="section-title" i18n="@@tools.collector.limits">Limits</legend>
        <div class="row">
          <label class="lbl"><ng-container i18n="@@tools.collector.maxBundle">Maximum bundle size (MB)</ng-container>
            <input class="input" type="number" min="1" [value]="plan().maxBundleMb" (input)="patch({ maxBundleMb: +val($event) })" />
          </label>
          <label class="lbl"><ng-container i18n="@@tools.collector.toolTimeout">Tool timeout (seconds)</ng-container>
            <input class="input" type="number" min="1" [value]="plan().commandTimeoutSecs" (input)="patch({ commandTimeoutSecs: +val($event) })" />
          </label>
        </div>
      </fieldset>
    }

    <label class="lbl"><ng-container i18n="@@tools.collector.outputFolder.label">Output folder</ng-container>
      <span class="inline">
        <input class="input grow mono" aria-label="Output folder" i18n-aria-label="@@tools.collector.outputFolder.aria" placeholder="Where to write the bundle" i18n-placeholder="@@tools.collector.outputFolder.placeholder" [value]="outDir()" (input)="outDir.set(val($event))" />
        <button type="button" class="btn" (click)="browse()" i18n="@@tools.collector.browse">Browse…</button>
      </span>
    </label>

    @if (phase() === 'running') {
      <div class="progress" role="status" data-testid="collect-progress" i18n="@@tools.collector.collecting"><span class="spinner" aria-hidden="true"></span> Collecting… this can take a minute on large log folders.</div>
      <p class="muted" i18n="@@tools.collector.closeHint">You can close this dialog: the collection goes on and is listed in Jobs.</p>
    }

    @if (result(); as r) {
      <div class="result" data-testid="collect-result">
        <strong i18n="@@tools.collector.bundleCreated">Bundle created</strong>
        <div class="mono path">{{ r.bundle }}</div>
        @if (summary(); as s) {
          <ul class="facts">
            <li i18n="@@tools.collector.facts.items"><strong>{{ fmt(s.items) }}</strong> {s.items, plural, =1 {item} other {items}} · {{ bytes(s.bytes) }}</li>
            <li><ng-container i18n="@@tools.collector.facts.masked"><strong>{{ fmt(s.redactionTotal) }}</strong> {s.redactionTotal, plural, =1 {value} other {values}} masked</ng-container>
              @if (s.redactionByRule.length) { ({{ ruleText() }}) }
            </li>
            <li i18n="@@tools.collector.facts.skipped"><strong>{{ s.skipped.length }}</strong> skipped</li>
            <li i18n="@@tools.collector.facts.errors"><strong>{{ s.errors.length }}</strong> {s.errors.length, plural, =1 {error} other {errors}}</li>
          </ul>
          @if (s.skipped.length) {
            <details open><summary i18n="@@tools.collector.skipped">Skipped</summary>
              <ul class="reasons">@for (k of s.skipped; track $index) { <li><span class="mono">{{ k.what }}</span>: {{ k.reason }}</li> }</ul>
            </details>
          }
          @if (s.errors.length) {
            <details open><summary i18n="@@tools.collector.errors">Errors</summary>
              <ul class="reasons">@for (e of s.errors; track $index) { <li class="bad">{{ e }}</li> }</ul>
            </details>
          }
        }
      </div>
    }

    <div class="foot">
      <button type="button" class="btn" (click)="ref.close()" i18n="@@tools.collector.close">Close</button>
      @if (result()) {
        <button type="button" class="btn" (click)="openBundle()" i18n="@@tools.collector.openBundle">Open bundle</button>
      }
      <button type="button" class="btn primary" [disabled]="phase() === 'running' || !!jsonError() || !canCollect()" (click)="collect()">{{ collectLabel() }}</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 12px; }
    .intro { margin: 0; color: var(--text-2); line-height: 1.5; }
    fieldset { border: 0; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; min-width: 0; }
    legend { padding: 8px 0 0; float: left; width: 100%; display: flex; align-items: center; gap: 6px; }
    legend + * { clear: both; }
    .bar { display: flex; justify-content: flex-end; }
    .lbl { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-xs); color: var(--muted); flex: 1; min-width: 0; }
    .row { display: flex; gap: 12px; flex-wrap: wrap; }
    .row .lbl { min-width: 140px; }
    .inline { display: flex; gap: 8px; align-items: center; }
    .grow { flex: 1; min-width: 0; }
    .checks { display: flex; flex-direction: column; gap: 2px; }
    /* 24px rows keep the 13px checkboxes 24px apart (WCAG 2.5.8 spacing). */
    .check { display: inline-flex; gap: 6px; align-items: center; min-height: 24px; color: var(--text-2); }
    .log { display: flex; flex-direction: column; gap: 6px; padding: 8px; border: 1px solid var(--border-2); border-radius: 6px; }
    .json { min-height: 280px; }
    .err { color: var(--danger-text); font-size: var(--fs-sm); }
    .progress { display: flex; gap: 8px; align-items: center; color: var(--text-2); }
    .result { padding: 10px 12px; border: 1px solid var(--ok); border-radius: 8px; background: var(--bg-3); display: flex; flex-direction: column; gap: 6px; font-size: var(--fs-sm); }
    .path { word-break: break-all; }
    .facts { list-style: none; padding: 0; margin: 0; display: flex; gap: 16px; flex-wrap: wrap; }
    .reasons { margin: 4px 0 0; padding-inline-start: 18px; }
    .bad { color: var(--danger-text); }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class CollectorDialog implements OnInit {
  protected readonly fmt = formatCount;
  protected readonly ref = inject(DIALOG_REF);
  private readonly backend = inject(CollectBackend);
  private readonly workspace = inject(WorkspaceStore);
  private readonly platform = inject(Platform);
  private readonly catalog = inject(ToolCatalog);
  private readonly overwrite = inject(OverwriteGuard);
  /** The tools plugin serves the collector now. */
  protected readonly canCollect = computed(() => this.catalog.usable(TOOL_IDS.diagnostics));

  protected readonly plan = signal<CollectPlan>(structuredClone(DEFAULT_PLAN));
  protected readonly jsonMode = signal(false);
  protected readonly jsonText = signal('');
  protected readonly jsonError = signal<string | null>(null);
  protected readonly outDir = signal('');
  protected readonly phase = signal<Phase>('idle');
  protected readonly error = signal<string | null>(null);
  protected readonly result = signal<CollectResult | null>(null);
  protected readonly summary = computed<ManifestSummary | null>(() => {
    const r = this.result();
    return r ? summarize(r.manifest) : null;
  });
  protected readonly ruleText = computed(() =>
    (this.summary()?.redactionByRule ?? []).map((r) => `${r.rule} ${r.count}`).join(', '),
  );
  protected readonly bytes = formatBytes;
  protected readonly collectLabel = computed(() =>
    this.result() ? $localize`:@@tools.collector.collectAgain:Collect again` : $localize`:@@tools.collector.collect:Collect`,
  );

  protected logFolderLabel = (i: number) => $localize`:@@tools.collector.logFolderN:Log folder ${i + 1}:n:`;
  protected removeLogLabel = (i: number) => $localize`:@@tools.collector.removeLogFolderN:Remove log folder ${i + 1}:n:`;
  protected includeLabel = (i: number) => $localize`:@@tools.collector.includeGlobsN:Include globs ${i + 1}:n:`;
  protected maxAgeLabel = (i: number) => $localize`:@@tools.collector.maxAgeN:Max age ${i + 1}:n:`;
  protected maxSizeLabel = (i: number) => $localize`:@@tools.collector.maxSizeN:Max size ${i + 1}:n:`;
  protected patternNameLabel = (i: number) => $localize`:@@tools.collector.patternNameN:Pattern name ${i + 1}:n:`;
  protected patternRegexLabel = (i: number) => $localize`:@@tools.collector.patternRegexN:Pattern regex ${i + 1}:n:`;
  protected patternReplaceLabel = (i: number) => $localize`:@@tools.collector.patternReplaceN:Pattern replacement ${i + 1}:n:`;
  protected removePatternLabel = (i: number) => $localize`:@@tools.collector.removePatternN:Remove pattern ${i + 1}:n:`;

  async ngOnInit(): Promise<void> {
    try {
      const parsed = parsePlan(await this.backend.collectDefaultPlan());
      if (parsed.ok) this.plan.set(parsed.plan);
      else this.error.set(parsed.error);
    } catch (e) {
      this.error.set($localize`:@@tools.collector.error.defaultPlan:Could not load the default plan: ${errorMessage(e)}:detail:`);
    }
  }

  protected val = (e: Event) => (e.target as HTMLInputElement).value;
  protected checked = (e: Event) => (e.target as HTMLInputElement).checked;
  protected num = (e: Event) => optionalNumber(this.val(e));
  protected split = (e: Event) => splitLines(this.val(e));
  protected csv = (e: Event) =>
    this.val(e)
      .split(',')
      .map((s) => s.trim())
      .filter(Boolean);
  protected lines = joinLines;

  protected patch(p: Partial<CollectPlan>): void {
    this.plan.update((cur) => ({ ...cur, ...p }));
  }
  protected patchRedact(p: Partial<CollectPlan['redact']>): void {
    this.plan.update((cur) => ({ ...cur, redact: { ...cur.redact, ...p } }));
  }
  protected addLog(): void {
    this.plan.update((cur) => ({ ...cur, logs: [...cur.logs, newLogSpec()] }));
  }
  protected setLog(index: number, p: Partial<LogSpec>): void {
    this.plan.update((cur) => ({ ...cur, logs: cur.logs.map((l, i) => (i === index ? { ...l, ...p } : l)) }));
  }
  protected removeLog(index: number): void {
    this.plan.update((cur) => ({ ...cur, logs: cur.logs.filter((_, i) => i !== index) }));
  }
  protected addPattern(): void {
    this.plan.update((cur) => ({ ...cur, redact: { ...cur.redact, patterns: [...cur.redact.patterns, { name: '', regex: '', replace: '[REDACTED]' }] } }));
  }
  protected setPattern(index: number, p: Partial<CustomPattern>): void {
    this.plan.update((cur) => ({ ...cur, redact: { ...cur.redact, patterns: cur.redact.patterns.map((x, i) => (i === index ? { ...x, ...p } : x)) } }));
  }
  protected removePattern(index: number): void {
    this.plan.update((cur) => ({ ...cur, redact: { ...cur.redact, patterns: cur.redact.patterns.filter((_, i) => i !== index) } }));
  }

  /** Switching to the form parses the JSON; a plan that does not parse keeps the JSON editor open. */
  protected toggleJson(): void {
    if (!this.jsonMode()) {
      this.jsonText.set(stringifyPlan(this.plan()));
      this.jsonError.set(null);
      this.jsonMode.set(true);
      return;
    }
    const parsed = parsePlan(this.jsonText());
    if (parsed.ok) {
      this.plan.set(parsed.plan);
      this.jsonMode.set(false);
    } else {
      this.jsonError.set(parsed.error);
    }
  }

  protected onJson(e: Event): void {
    const text = this.val(e);
    this.jsonText.set(text);
    const parsed = parsePlan(text);
    this.jsonError.set(parsed.ok ? null : parsed.error);
    if (parsed.ok) this.plan.set(parsed.plan);
  }

  protected async browse(): Promise<void> {
    try {
      const [dir] = await this.platform.pickPaths(true);
      if (dir) this.outDir.set(dir);
    } catch (e) {
      this.error.set(errorMessage(e));
    }
  }

  protected async collect(): Promise<void> {
    const dir = this.outDir().trim();
    if (!dir) {
      this.error.set($localize`:@@tools.collector.error.chooseFolder:Choose an output folder for the bundle.`);
      return;
    }
    this.phase.set('running');
    this.error.set(null);
    this.result.set(null);
    try {
      // The engine parses and validates the plan; the JSON text is sent exactly as the form/JSON editor shows it.
      // The bundle name is the app's, so a bundle that already exists is replaced only after asking.
      const plan = JSON.stringify(this.plan());
      const result = await this.overwrite.write(dir, (overwrite) => this.backend.collectDiagnostics(plan, dir, overwrite), 'app');
      this.result.set(result);
      this.phase.set(result ? 'done' : 'idle');
    } catch (e) {
      this.error.set(errorMessage(e));
      this.phase.set('idle');
    }
  }

  protected async openBundle(): Promise<void> {
    const bundle = this.result()?.bundle;
    if (!bundle) return;
    await this.workspace.openPaths([bundle]);
    if (!this.workspace.error()) this.ref.close();
  }
}
