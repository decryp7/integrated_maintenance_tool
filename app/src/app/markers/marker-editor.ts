import { ChangeDetectionStrategy, Component, computed, inject, input, linkedSignal, output, signal } from '@angular/core';
import { formatCount } from '../core/format';
import { Condition } from '../core/profile.model';
import { WorkspaceStore } from '../core/workspace.store';
import { slug } from '../context/rule-builders';
import { DIALOG_FORM_STYLES } from '../context/dialog-styles';
import { ConditionsEditor } from '../profile/conditions-editor';
import { builtinIcon } from './builtin-icons';
import { GlyphMode, GlyphPicker } from './glyph-picker';
import { qlToConditions } from './highlight';
import { cleanRule, nextFreeBadge, ruleProblems, uniqueKind, upsertIcon, pruneIcons, writeRows } from './marker-rules';
import { MarkerGlyph } from './marker-glyph';
import { MarkerProfile } from './marker-profile';
import { livePreview } from './marker-preview';
import { GlyphStyle, MarkerRuleX, PendingIcon, RuleRow, SaveFailure } from './markers.types';
import { RegexAssist } from './regex/regex-assist';

export const COLOR_PRESETS = ['#f2545b', '#e9ae3f', '#3fbf7f', '#3987e5', '#9085e9', '#ff4fa3', '#2fb8c6', '#a7b0c2'];

/**
 * The one editor for a marker rule: used by "Manage markers", "Highlight lines…" and
 * "Mark events like this…". Matching, validation and saving are done by the engine; this form only
 * collects the rule and previews what it matches.
 */
@Component({
  selector: 'debrief-marker-editor',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ConditionsEditor, GlyphPicker, MarkerGlyph, RegexAssist],
  template: `
    <div class="top">
      <debrief-marker-glyph [glyph]="style()" [size]="32" [label]="previewLabel" />
      <div class="grow">
        <div class="row">
          <label class="lbl"><ng-container i18n="@@markers.editor.label">Label</ng-container><input class="input" [value]="label()" (input)="label.set($any($event.target).value)" /></label>
          <label class="lbl"><ng-container i18n="@@markers.editor.kind">Kind (id)</ng-container><input class="input mono" [value]="kind()" (input)="kindEdited.set($any($event.target).value)" /></label>
        </div>
        <label class="lbl"><ng-container i18n="@@markers.editor.description">Description (shown in the legend and tooltips)</ng-container><input class="input" [value]="description()" placeholder="What does this marker mean?" i18n-placeholder="@@markers.editor.descriptionPlaceholder" (input)="description.set($any($event.target).value)" /></label>
      </div>
    </div>

    <debrief-glyph-picker [(mode)]="mode" [(badge)]="badge" [(shape)]="shape" [(icon)]="icon" [(pending)]="pending" [color]="color()" [suggestion]="suggestion()" [takenNames]="takenIcons()" />

    <div class="row">
      <label class="lbl"><ng-container i18n="@@markers.editor.colour">Colour</ng-container><input class="input color" type="color" [value]="color()" (input)="color.set($any($event.target).value)" /></label>
      <div class="presets" role="group" aria-label="Colour presets" i18n-aria-label="@@markers.editor.colourPresets">
        @for (p of presets; track p) {
          <button type="button" class="swatch" [style.background]="p" [attr.aria-label]="colourLabel(p)" [attr.aria-pressed]="color() === p" (click)="color.set(p)"></button>
        }
      </div>
    </div>
    <label class="check" i18n="@@markers.editor.lifecycle"><input type="checkbox" [checked]="lifecycle()" (change)="lifecycle.set($any($event.target).checked)" /> Lifecycle marker (start/stop: stays visible even when filters hide it)</label>

    @if (advanced()) {
      <label class="lbl"><ng-container i18n="@@markers.editor.queryLanguage">Query language (preview; saved as conditions when simple)</ng-container>
        <textarea class="input mono" rows="2" [value]="ql()" (input)="ql.set($any($event.target).value)"></textarea>
      </label>
      <div class="row">
        <button type="button" class="btn" [disabled]="!converted()" (click)="convert()" i18n="@@markers.editor.convert">Convert to conditions</button>
        <button type="button" class="btn ghost" (click)="advanced.set(false)" i18n="@@markers.editor.backToBuilder">Back to the condition builder</button>
        @if (!converted()) { <span class="faint" i18n="@@markers.editor.tooComplex">This query is too complex for conditions, so it can only be previewed.</span> }
      </div>
    } @else {
      <debrief-conditions-editor [conditions]="conditions()" emptyText="Add a condition: a marker without any would mark every event." i18n-emptyText="@@markers.editor.emptyConditions" (changed)="conditions.set($event)" />
      @for (c of regexConditions(); track $index) {
        <div class="assist"><span class="faint mono ellipsis">{{ c.value }}</span><debrief-regex-assist [pattern]="c.value ?? ''" /></div>
      }
      <button type="button" class="btn ghost link" (click)="advanced.set(true)" i18n="@@markers.editor.advanced">Advanced: query language</button>
    }

    <section class="preview" aria-label="Preview" i18n-aria-label="@@markers.editor.preview">
      <div class="status" role="status">
        @if (preview().error; as e) { <ng-container i18n="@@markers.editor.couldNotCount">Could not count: {{ e }}</ng-container> }
        @else if (preview().result; as r) { <ng-container i18n="@@markers.editor.marksNow">This marks <strong>{{ fmt(r.count) }}</strong> events right now.</ng-container>&ngsp;@if (r.ignored) { <span class="faint" i18n="@@markers.editor.ignoredIn">({{ r.ignored }} "in" condition ignored in the preview)</span> } }
        @else { <ng-container i18n="@@markers.editor.counting">Counting…</ng-container> }
      </div>
      @if (preview().result; as r) {
        <svg class="strip" viewBox="0 0 48 12" preserveAspectRatio="none" role="img" aria-label="Where the matches are in time" i18n-aria-label="@@markers.editor.strip">
          @for (v of r.strip; track $index) {
            @if (v) { <rect [attr.x]="$index" [attr.y]="12 - h(v, r.strip)" width="0.8" [attr.height]="h(v, r.strip)" /> }
          }
        </svg>
        <ol class="samples" aria-label="First matching lines" i18n-aria-label="@@markers.editor.samples">
          @for (s of r.samples; track s.id) {
            <li><span class="faint mono">{{ s.level }}</span> {{ s.message }}</li>
          }
        </ol>
      }
    </section>

    @if (failure(); as f) {
      <div class="err" role="alert">{{ f.message }}@if (f.line) { <span> (<ng-container i18n="@@markers.editor.profileLine">profile line {{ f.line }}</ng-container>@if (f.excerpt) { : <code>{{ f.excerpt }}</code> })</span> }</div>
    }
    @if (problems().length && attempted()) { <div class="err" role="alert">{{ problems().join(' ') }}</div> }
    <div class="foot">
      <button type="button" class="btn ghost" (click)="cancelled.emit()" i18n="@@markers.editor.cancel">Cancel</button>
      <button type="button" class="btn primary" [disabled]="saving() || !hasConditions()" (click)="save()">{{ saveLabel() }}</button>
    </div>
  `,
  styles: [
    DIALOG_FORM_STYLES,
    `
    .top { display: flex; gap: 12px; align-items: flex-start; }
    .grow { flex: 1; display: flex; flex-direction: column; gap: 8px; min-width: 0; }
    .color { width: 56px; padding: 2px; }
    .presets { display: flex; gap: 4px; padding-bottom: 6px; }
    .swatch { width: 22px; height: 22px; border-radius: 50%; border: 2px solid var(--bg-1); outline: 1px solid var(--border-3); cursor: pointer; }
    .swatch[aria-pressed='true'] { outline: 2px solid var(--focus-ring); }
    .assist { display: flex; align-items: flex-start; gap: 8px; font-size: var(--fs-sm); }
    .assist .ellipsis { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; padding-top: 3px; }
    .link { align-self: flex-start; }
    .preview { display: flex; flex-direction: column; gap: 6px; }
    .strip { width: 100%; height: 28px; background: var(--code-bg); border: 1px solid var(--border); border-radius: 4px; }
    .strip rect { fill: var(--accent); }
    .samples { margin: 0; padding: 6px 8px; list-style: none; max-height: 150px; overflow: auto; font-size: var(--fs-sm); background: var(--code-bg); border: 1px solid var(--border); border-radius: 6px; display: flex; flex-direction: column; gap: 2px; }
    .samples li { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    textarea { resize: vertical; }
    code { font-family: var(--font-mono); }
  `,
  ],
})
export class MarkerEditor {
  private readonly profile = inject(MarkerProfile);
  private readonly store = inject(WorkspaceStore);

  /** Conditions to start from (new rule). */
  readonly initial = input<Condition[]>([]);
  readonly suggestedLabel = input($localize`:@@markers.editor.defaultLabel:Highlighted lines`);
  readonly suggestedDescription = input('');
  /** The rule being edited; null creates a new one. */
  readonly rule = input<MarkerRuleX | null>(null);
  readonly submitLabel = input($localize`:@@markers.editor.add:Add marker`);
  readonly saved = output<void>();
  readonly cancelled = output<void>();

  protected readonly previewLabel = $localize`:@@markers.editor.previewLabel:Marker preview`;
  protected readonly presets = COLOR_PRESETS;
  protected readonly saveLabel = computed(() => (this.saving() ? $localize`:@@markers.editor.applying:Applying…` : this.submitLabel()));
  protected readonly fmt = formatCount;
  protected readonly rows = signal<RuleRow[]>([]);

  protected readonly label = linkedSignal(() => this.rule()?.label ?? this.rule()?.kind ?? this.suggestedLabel());
  protected readonly description = linkedSignal(() => this.rule()?.description ?? this.suggestedDescription());
  protected readonly kindEdited = linkedSignal<string | null>(() => this.rule()?.kind ?? null);
  protected readonly kind = computed(() => this.kindEdited() ?? uniqueKind(this.rows(), slug(this.label())));
  protected readonly color = linkedSignal(() => this.rule()?.color ?? '#e9ae3f');
  protected readonly lifecycle = linkedSignal(() => !!this.rule()?.lifecycle);
  protected readonly conditions = linkedSignal<Condition[]>(() => this.rule()?.when ?? this.initial());
  protected readonly mode = linkedSignal<GlyphMode>(() => {
    const r = this.rule();
    if (!r?.icon || r.badge) return 'badge';
    return builtinIcon(r.icon) ? 'icon' : 'custom';
  });
  protected readonly badge = linkedSignal(() => (this.rule() ? (this.rule()!.badge ?? '') : this.suggestion()));
  protected readonly shape = linkedSignal(() => this.rule()?.shape ?? 'circle');
  protected readonly icon = linkedSignal(() => this.rule()?.icon ?? '');
  protected readonly pending = signal<PendingIcon | null>(null);
  protected readonly advanced = signal(false);
  protected readonly ql = signal('');
  protected readonly saving = signal(false);
  protected readonly attempted = signal(false);
  protected readonly failure = signal<SaveFailure | null>(null);

  protected readonly suggestion = computed(() => nextFreeBadge(this.rows()));
  protected readonly takenIcons = computed(() => new Set(this.rows().map((r) => r.rule.icon).filter((i): i is string => !!i)));
  protected readonly regexConditions = computed(() => this.conditions().filter((c) => c.op === 'matches' && c.value));
  protected readonly converted = computed(() => qlToConditions(this.ql()));
  protected readonly preview = livePreview(() => (this.advanced() ? { conditions: [], ql: this.ql() } : { conditions: this.conditions() }));

  protected readonly style = computed<GlyphStyle>(() => {
    const p = this.pending();
    const base: GlyphStyle = { kind: this.kind(), color: this.color() };
    if (this.mode() === 'badge') return { ...base, badge: this.badge(), shape: this.shape() };
    if (this.mode() === 'custom' && p) return { ...base, svg: p.svg, pngDataUrl: p.pngDataUrl };
    return { ...base, icon: this.icon() };
  });

  protected readonly hasConditions = computed(() => (this.advanced() ? !!this.converted()?.length : this.conditions().length > 0));
  protected readonly problems = computed(() => ruleProblems(this.buildRule()));

  constructor() {
    void this.profile.load().then(({ rows }) => this.rows.set(rows));
  }

  protected colourLabel(colour: string): string {
    return $localize`:@@markers.editor.colourAria:Colour ${colour}:colour:`;
  }

  protected h(v: number, strip: number[]): number {
    return Math.max(1, (v / Math.max(...strip, 1)) * 11);
  }

  protected convert(): void {
    const c = this.converted();
    if (!c) return;
    this.conditions.set(c);
    this.advanced.set(false);
  }

  private buildRule(): MarkerRuleX {
    const mode = this.mode();
    const rule: MarkerRuleX = {
      kind: this.kind(),
      label: this.label(),
      description: this.description().trim(),
      color: this.color(),
      lifecycle: this.lifecycle(),
      when: this.advanced() ? (this.converted() ?? []) : this.conditions(),
    };
    if (mode === 'badge') Object.assign(rule, { badge: this.badge().trim(), shape: this.shape() === 'circle' ? undefined : this.shape() });
    else if (this.icon()) rule.icon = this.icon();
    // Keep unknown keys of the rule being edited (e.g. keys written by other tools).
    const old = this.rule();
    const merged = { ...(old ?? {}), ...rule } as MarkerRuleX;
    if (mode === 'badge') delete merged.icon;
    else delete merged.badge;
    if (mode !== 'badge') delete merged.shape;
    return cleanRule(merged);
  }

  protected async save(): Promise<void> {
    this.attempted.set(true);
    if (this.problems().length) return;
    this.saving.set(true);
    this.failure.set(null);
    const rule = this.buildRule();
    const original = this.rule()?.kind;
    const pending = this.mode() === 'custom' ? this.pending() : null;
    const result = await this.profile.commit((model, rows) => {
      const i = original === undefined ? -1 : rows.findIndex((r) => r.rule.kind === original);
      const kind = i >= 0 ? rule.kind : uniqueKind(rows, rule.kind);
      const next = { ...rule, kind };
      const row = i >= 0 ? rows[i] : undefined;
      if (row) rows[i] = { ...row, rule: next };
      else rows.unshift({ rule: next, enabled: true }); // user highlights win over the defaults
      if (pending) upsertIcon(model, pending);
      writeRows(model, rows);
      pruneIcons(model, rows);
    });
    this.saving.set(false);
    if (!result.ok) {
      this.failure.set(result.failure);
      return;
    }
    const name = rule.label || rule.kind;
    this.store.setNotice(
      original === undefined
        ? $localize`:@@markers.editor.added:Added the "${name}:name:" marker; files were re-imported.`
        : $localize`:@@markers.editor.updated:Updated the "${name}:name:" marker; files were re-imported.`,
    );
    this.saved.emit();
  }
}
