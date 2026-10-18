import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, DestroyRef, effect, inject, signal, untracked } from '@angular/core';
import { AnnotationDto, SpanCfgToml, SpanField, SpanPreview, SpanSummary } from '../core/api.types';
import { formatCount, formatDateTime, formatDuration } from '../core/format';
import { describeError } from '../core/logger';
import { WorkspaceStore } from '../core/workspace.store';
import { RunsBackend } from '../timeline/runs-backend';
import { runDuration, runStatusLabel } from '../timeline/run-text';
import { MarkerProfile } from './marker-profile';
import { applyPlan, emptyForm, fromRule, PresetId, presets, RunForm, slug, toDraft, TriggerForm, withEnabled, without } from './runs-form';

/** Wait after the last keystroke before asking the engine for a preview. */
const PREVIEW_DEBOUNCE_MS = 300;

/**
 * The "Runs" section of the Markers view: the run rules of the profile, and a guided form — "a run starts when …
 * and stops when …, separately for each …" — with presets, a live preview over the loaded data and the engine's
 * validation messages next to the fields. Saving writes the rule (and the marker rules its text patterns need) into
 * the profile through the same validate-and-save flow as marker rules.
 */
@Component({
  selector: 'imt-runs-editor',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <section class="runs" aria-labelledby="runs-title">
      <h3 class="sub" id="runs-title"><span i18n="@@runs.editor.title">Runs</span> <span class="faint" i18n="@@runs.editor.intro">— the time between a start and a stop, drawn as a band on each timeline lane.</span></h3>

      @if (rules().length) {
        <ul class="rules" aria-label="Run rules" i18n-aria-label="@@runs.editor.rulesAria">
          @for (r of rules(); track r.kind) {
            <li [class.off]="r.enabled === false" [attr.data-kind]="r.kind">
              <strong>{{ r.label || r.kind }}</strong>
              <span class="faint">{{ ruleText(r) }}</span>
              @if (summaryOf(r.kind); as s) { <span class="faint mono">{{ countText(s) }}</span> }
              <span class="spacer"></span>
              <label class="check"><input type="checkbox" [checked]="r.enabled !== false" (change)="toggle(r)" /><span i18n="@@runs.editor.on">On</span></label>
              <button type="button" class="btn" [attr.aria-label]="editAria(r)" (click)="edit(r)" i18n="@@runs.editor.edit">Edit</button>
              @if (confirming() === r.kind) {
                <button type="button" class="btn danger" (click)="remove(r)" i18n="@@runs.editor.confirmDelete">Confirm delete</button>
                <button type="button" class="btn ghost" (click)="confirming.set(null)" i18n="@@runs.editor.keep">Keep</button>
              } @else {
                <button type="button" class="btn ghost" [attr.aria-label]="deleteAria(r)" (click)="confirming.set(r.kind)" i18n="@@runs.editor.delete">Delete</button>
              }
            </li>
          }
        </ul>
      }

      @if (!open()) {
        <button type="button" class="btn primary" data-testid="runs-new" (click)="start()" i18n="@@runs.editor.new">New run rule…</button>
      } @else {
        <form class="form" data-testid="runs-form" (submit)="$event.preventDefault(); save()" aria-labelledby="runs-form-title">
          <h4 id="runs-form-title">@if (form().editing) { <ng-container i18n="@@runs.editor.editTitle">Edit run rule</ng-container> } @else { <ng-container i18n="@@runs.editor.newTitle">New run rule</ng-container> }</h4>
          <fieldset class="presets">
            <legend i18n="@@runs.editor.presets">Start from</legend>
            @for (p of presetList; track p.id) {
              <label class="preset" [title]="p.hint"><input type="radio" name="preset" [value]="p.id" [checked]="preset() === p.id" (change)="choosePreset(p.id)" />{{ p.label }}</label>
            }
          </fieldset>

          <label class="row">
            <span class="lbl" i18n="@@runs.editor.name">Name</span>
            <input class="input" name="label" [value]="form().label" maxlength="200" (input)="setLabel(text($event))" [attr.aria-describedby]="describedBy('kind')" />
            <span class="faint mono small">{{ draft().kind }}</span>
          </label>
          @if (issue('kind') ?? issue('label'); as m) { <div class="err" id="runs-err-kind" role="alert">{{ m }}</div> }

          <div class="sentence" role="group" aria-label="Rule" i18n-aria-label="@@runs.editor.ruleAria">
            <span i18n="@@runs.editor.startsWhen">A run starts when</span>
            <ng-container [ngTemplateOutlet]="trig" [ngTemplateOutletContext]="{ side: 'start', t: form().start }" />
            <span i18n="@@runs.editor.stopsWhen">and stops when</span>
            <ng-container [ngTemplateOutlet]="trig" [ngTemplateOutletContext]="{ side: 'stop', t: form().stop }" />
            <span i18n="@@runs.editor.separately">separately for each</span>
            <select class="select" name="per" aria-label="Pair starts and stops within" i18n-aria-label="@@runs.editor.perAria" [attr.aria-describedby]="describedBy('per')" (change)="patch({ field: text($event) })">
              <option value="" [selected]="!form().field" i18n="@@runs.editor.lane">timeline lane</option>
              @for (f of fieldOptions(); track f) { <option [value]="f" [selected]="form().field === f">{{ f }}</option> }
            </select>
          </div>
          @if (issue('start'); as m) { <div class="err" id="runs-err-start" role="alert">{{ m }}</div> }
          @if (issue('stop'); as m) { <div class="err" id="runs-err-stop" role="alert">{{ m }}</div> }
          @if (issue('per'); as m) { <div class="err" id="runs-err-per" role="alert">{{ m }}</div> }

          <details class="advanced">
            <summary i18n="@@runs.editor.advanced">More options</summary>
            <label class="row"><span class="lbl" i18n="@@runs.editor.maxDuration">Flag runs longer than</span><input class="input short" name="maxDuration" placeholder="e.g. 12h" i18n-placeholder="@@runs.editor.maxPlaceholder" [value]="form().maxDuration" (input)="patch({ maxDuration: text($event) })" [attr.aria-describedby]="describedBy('maxDuration')" /></label>
            @if (issue('maxDuration'); as m) { <div class="err" id="runs-err-maxDuration" role="alert">{{ m }}</div> }
            <label class="row check"><input type="checkbox" name="restart" [checked]="form().restartOnStart" (change)="patch({ restartOnStart: checked($event) })" /><span i18n="@@runs.editor.restart">A new start ends a run that has no stop (shown as "no stop seen")</span></label>
            <label class="row"><span class="lbl" i18n="@@runs.editor.color">Band colour</span><input class="input short" name="color" placeholder="#3987e5" [value]="form().color" (input)="patch({ color: text($event) })" [attr.aria-describedby]="describedBy('color')" /></label>
            @if (issue('color'); as m) { <div class="err" id="runs-err-color" role="alert">{{ m }}</div> }
          </details>

          @if (issue('general'); as m) { <div class="err" role="alert">{{ m }}</div> }
          @for (w of preview()?.warnings ?? []; track w) { <div class="warn">{{ w }}</div> }

          <div class="preview" aria-live="polite" data-testid="runs-preview">
            @if (previewError(); as e) {
              <span class="err">{{ e }}</span>
            } @else if (preview()?.summary; as s) {
              <div class="counts">{{ previewText(s) }}</div>
              @if (anomalyCount(s); as n) { <div class="warn" i18n="@@runs.editor.anomalies">⚠ {{ n //i18n(ph="count") }} anomalies (no start, no stop or too long)</div> }
              @if (previewRows().length) {
                <table class="mini" aria-label="First runs" i18n-aria-label="@@runs.editor.firstRuns">
                  <thead><tr><th scope="col" i18n="@@runs.editor.col.lane">Lane</th><th scope="col" i18n="@@runs.editor.col.start">Start</th><th scope="col" i18n="@@runs.editor.col.duration">Duration</th><th scope="col" i18n="@@runs.editor.col.status">Status</th></tr></thead>
                  <tbody>
                    @for (a of previewRows(); track $index) {
                      <tr><td class="mono">{{ a.instance }}</td><td class="mono">{{ time(a.from) }}</td><td class="mono">{{ dur(a) }}</td><td>{{ status(a) }}</td></tr>
                    }
                  </tbody>
                </table>
              }
            } @else if (checking()) {
              <span class="faint" i18n="@@runs.editor.checking">Checking…</span>
            }
          </div>

          @if (saveError(); as e) { <div class="err" role="alert">{{ e }}</div> }
          <div class="actions">
            <button type="submit" class="btn primary" data-testid="runs-save" [disabled]="!canSave() || saving()" i18n="@@runs.editor.save">Save run rule</button>
            <button type="button" class="btn ghost" (click)="cancel()" i18n="@@runs.editor.cancel">Cancel</button>
          </div>
        </form>
      }
    </section>

    <ng-template #trig let-side="side" let-t="t">
      <select class="select" [attr.name]="side + '-mode'" [attr.aria-label]="side === 'start' ? startAria : stopAria" [attr.aria-describedby]="describedBy(side)" (change)="setTrigger(side, triggerFrom(text($event), t))">
        <optgroup label="Marker" i18n-label="@@runs.editor.markerGroup">
          @for (k of markerOptions(); track k.kind) { <option [value]="'m:' + k.kind" [selected]="t.mode === 'marker' && t.marker === k.kind">{{ k.label }}</option> }
        </optgroup>
        <option value="p:" [selected]="t.mode === 'pattern'" i18n="@@runs.editor.textMatches">the message matches…</option>
      </select>
      @if (t.mode === 'pattern') {
        <input class="input mono pattern" [attr.name]="side + '-pattern'" [attr.aria-label]="side === 'start' ? startPatternAria : stopPatternAria" placeholder="e.g. worker started" i18n-placeholder="@@runs.editor.patternPlaceholder" maxlength="4000" [value]="t.pattern" (input)="setTrigger(side, { mode: 'pattern', marker: '', pattern: text($event) })" />
      }
    </ng-template>
  `,
  imports: [NgTemplateOutlet],
  styles: `
    :host { display: block; }
    .sub { font-size: 12.5px; font-weight: 600; margin: 18px 0 6px; }
    .sub .faint { font-weight: 400; }
    h4 { margin: 0 0 6px; font-size: 12.5px; }
    .rules { list-style: none; margin: 0 0 8px; padding: 0; font-size: 12.5px; }
    .rules li { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 10px; padding: 6px 8px; border-bottom: 1px solid var(--border); }
    .rules li.off { opacity: 0.6; }
    .spacer { flex: 1; }
    .check { display: inline-flex; align-items: center; gap: 4px; }
    .form { display: flex; flex-direction: column; gap: 8px; max-width: 900px; padding: 10px 12px; border: 1px solid var(--border); border-radius: 6px; background: var(--bg-2); font-size: 12.5px; }
    .presets { display: flex; flex-wrap: wrap; gap: 4px 16px; border: 0; margin: 0; padding: 0; }
    .presets legend { padding: 0; margin-bottom: 4px; color: var(--muted); }
    .preset { display: inline-flex; align-items: center; gap: 4px; min-height: 24px; }
    .row { display: flex; align-items: center; gap: 8px; }
    .lbl { min-width: 160px; color: var(--muted); }
    .input { height: 26px; padding: 0 6px; min-width: 220px; }
    .input.short { min-width: 120px; width: 120px; }
    .input.pattern { min-width: 260px; }
    .select { height: 26px; }
    .small { font-size: 11px; }
    .sentence { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 8px; }
    .err { color: var(--danger-text); }
    .warn { color: var(--warn-text); }
    .advanced summary { cursor: pointer; color: var(--muted); min-height: 24px; display: list-item; }
    .advanced .row { margin-top: 6px; }
    .preview { min-height: 22px; }
    .counts { color: var(--text); }
    .mini { border-collapse: collapse; margin-top: 4px; font-size: 12px; }
    .mini th, .mini td { text-align: left; padding: 2px 10px 2px 0; border-bottom: 1px solid var(--border); white-space: nowrap; }
    .mini th { color: var(--muted); font-weight: 500; }
    .actions { display: flex; gap: 8px; }
  `,
})
export class RunsEditor {
  private readonly backend = inject(RunsBackend);
  private readonly profile = inject(MarkerProfile);
  private readonly store = inject(WorkspaceStore);

  protected readonly presetList = presets();
  protected readonly startAria = $localize`:@@runs.editor.startAria:What starts a run`;
  protected readonly stopAria = $localize`:@@runs.editor.stopAria:What stops a run`;
  protected readonly startPatternAria = $localize`:@@runs.editor.startPatternAria:Text that starts a run (regex)`;
  protected readonly stopPatternAria = $localize`:@@runs.editor.stopPatternAria:Text that stops a run (regex)`;

  protected readonly rules = signal<SpanCfgToml[]>([]);
  protected readonly summaries = signal<SpanSummary[]>([]);
  protected readonly confirming = signal<string | null>(null);
  protected readonly open = signal(false);
  protected readonly preset = signal<PresetId>('service');
  protected readonly form = signal<RunForm>(emptyForm());
  protected readonly preview = signal<SpanPreview | null>(null);
  protected readonly previewError = signal<string | null>(null);
  protected readonly checking = signal(false);
  protected readonly saving = signal(false);
  protected readonly saveError = signal<string | null>(null);

  protected readonly draft = computed(() => toDraft(this.form()));
  protected readonly markerOptions = computed(() => this.store.summary()?.markerKinds ?? []);
  protected readonly fieldOptions = computed(() => {
    const f = this.form().field;
    const names = this.store.fieldNames();
    return f && !names.includes(f) ? [f, ...names] : names;
  });
  protected readonly canSave = computed(() => {
    const p = this.preview();
    return !!p && !p.errors.length && !!p.plan && !this.checking();
  });
  protected readonly previewRows = computed(() => {
    const p = this.preview();
    if (!p) return [];
    const anomalies = p.anomalies.filter((a) => !p.runs.includes(a)).slice(0, 5);
    return [...p.runs.slice(0, 10), ...anomalies];
  });

  private seq = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor() {
    effect(() => {
      this.store.summary();
      untracked(() => void this.reload());
    });
    // Preview the draft after each change (debounced); superseded answers are ignored.
    effect(() => {
      const draft = this.draft();
      if (!this.open()) return;
      untracked(() => this.schedule(draft));
    });
    inject(DestroyRef).onDestroy(() => {
      if (this.timer) clearTimeout(this.timer);
    });
  }

  private async reload(): Promise<void> {
    try {
      const reply = await this.backend.queryRuns({ limit: 1 });
      this.rules.set(reply.rules ?? []);
      this.summaries.set(reply.summaries ?? []);
    } catch {
      /* the list stays as it was; the Runs view reports load failures */
    }
  }

  private schedule(draft: ReturnType<typeof toDraft>): void {
    if (this.timer) clearTimeout(this.timer);
    const mine = ++this.seq;
    this.checking.set(true);
    this.timer = setTimeout(() => void this.runPreview(draft, mine), PREVIEW_DEBOUNCE_MS);
  }

  private async runPreview(draft: ReturnType<typeof toDraft>, mine: number): Promise<void> {
    try {
      const p = await this.backend.previewSpans({ draft });
      if (mine !== this.seq) return;
      this.preview.set(p);
      this.previewError.set(null);
    } catch (e) {
      if (mine !== this.seq) return;
      this.preview.set(null);
      this.previewError.set($localize`:@@runs.editor.previewFailed:The preview failed: ${describeError(e)}:error:`);
    } finally {
      if (mine === this.seq) this.checking.set(false);
    }
  }

  protected issue(field: SpanField): string | null {
    return this.preview()?.errors.find((e) => e.field === field)?.message ?? null;
  }

  /** The id of a field's error message, when it has one. */
  protected describedBy(field: SpanField): string | null {
    return this.issue(field) ? 'runs-err-' + field : null;
  }

  protected text(e: Event): string {
    return (e.target as HTMLInputElement | HTMLSelectElement).value;
  }

  protected checked(e: Event): boolean {
    return (e.target as HTMLInputElement).checked;
  }

  protected patch(p: Partial<RunForm>): void {
    this.form.update((f) => ({ ...f, ...p }));
  }

  protected setLabel(label: string): void {
    this.form.update((f) => ({ ...f, label, kind: f.editing ? f.kind : slug(label) }));
  }

  protected setTrigger(side: 'start' | 'stop', t: TriggerForm): void {
    this.preset.set('custom');
    this.form.update((f) => ({ ...f, [side]: t }));
  }

  /** The trigger a select value means (`m:<kind>` or `p:`); a pattern keeps its text. */
  protected triggerFrom(value: string, current: TriggerForm): TriggerForm {
    return value.startsWith('m:') ? { mode: 'marker', marker: value.slice(2), pattern: current.pattern } : { mode: 'pattern', marker: '', pattern: current.pattern };
  }

  protected choosePreset(id: PresetId): void {
    this.preset.set(id);
    const p = this.presetList.find((x) => x.id === id);
    if (p) this.form.update((f) => p.apply(f));
  }

  protected start(): void {
    const base = emptyForm();
    const first = this.presetList[0];
    this.form.set(first ? first.apply(base) : base);
    this.preset.set('service');
    this.preview.set(null);
    this.saveError.set(null);
    this.open.set(true);
  }

  protected edit(r: SpanCfgToml): void {
    this.form.set(fromRule(r));
    this.preset.set('custom');
    this.preview.set(null);
    this.saveError.set(null);
    this.open.set(true);
  }

  protected cancel(): void {
    this.open.set(false);
    this.preview.set(null);
    this.seq++;
  }

  protected async save(): Promise<void> {
    const plan = this.preview()?.plan;
    if (!plan || !this.canSave()) return;
    this.saving.set(true);
    const res = await this.profile.commit((model) => applyPlan(model, plan), $localize`:Undo history entry|@@runs.editor.undoLabel:run rule edit`);
    this.saving.set(false);
    if (res.ok) {
      this.open.set(false);
      this.saveError.set(null);
    } else this.saveError.set(res.failure.line ? `${res.failure.message} (line ${res.failure.line})` : res.failure.message);
  }

  protected async toggle(r: SpanCfgToml): Promise<void> {
    await this.commitRules(withEnabled(this.rules(), r.kind, r.enabled === false));
  }

  protected async remove(r: SpanCfgToml): Promise<void> {
    this.confirming.set(null);
    await this.commitRules(without(this.rules(), r.kind));
  }

  private async commitRules(rules: SpanCfgToml[]): Promise<void> {
    const res = await this.profile.commit((model) => {
      model.span = rules;
    }, $localize`:Undo history entry|@@runs.editor.undoLabel:run rule edit`);
    if (!res.ok) this.saveError.set(res.failure.message);
  }

  protected summaryOf(kind: string): SpanSummary | undefined {
    return this.summaries().find((s) => s.kind === kind);
  }

  protected ruleText(r: SpanCfgToml): string {
    const start = Array.isArray(r.start) ? r.start.join(' / ') : r.start;
    const stop = Array.isArray(r.stop) ? r.stop.join(' / ') : r.stop;
    const per = r.per && r.per !== 'instance' ? r.per.replace(/^field:\s*/, '') : $localize`:@@runs.editor.laneWord:lane`;
    return $localize`:@@runs.editor.ruleText:${start}:start: → ${stop}:stop:, per ${per}:per:`;
  }

  protected countText(s: SpanSummary): string {
    const runs = formatCount(s.runs);
    const anomalies = formatCount(s.noStop + s.noStart + s.tooLong);
    return $localize`:@@runs.editor.countText:${runs}:runs: runs, ${anomalies}:anomalies: anomalies`;
  }

  protected previewText(s: SpanSummary): string {
    const runs = formatCount(s.runs);
    const lanes = formatCount(s.lanes);
    const closed = formatCount(s.closed);
    const open = formatCount(s.open);
    const avg = formatDuration(s.avgMs);
    const max = formatDuration(s.maxMs);
    return $localize`:@@runs.editor.previewText:${runs}:runs: runs on ${lanes}:lanes: lanes in the loaded data: ${closed}:closed: stopped, ${open}:open: still running · average ${avg}:avg:, longest ${max}:max:`;
  }

  protected anomalyCount(s: SpanSummary): number {
    return s.noStop + s.noStart + s.tooLong;
  }

  protected editAria(r: SpanCfgToml): string {
    const name = r.label || r.kind;
    return $localize`:@@runs.editor.editAria:Edit ${name}:name:`;
  }

  protected deleteAria(r: SpanCfgToml): string {
    const name = r.label || r.kind;
    return $localize`:@@runs.editor.deleteAria:Delete ${name}:name:`;
  }

  protected time(ts: number): string {
    return formatDateTime(ts, this.store.timezone(), true);
  }

  protected dur(a: AnnotationDto): string {
    return a.run ? runDuration(a.run) : '–';
  }

  protected status(a: AnnotationDto): string {
    return a.run ? runStatusLabel(a.run.status) : '';
  }
}
