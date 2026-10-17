import { ChangeDetectionStrategy, Component, computed, effect, inject, input, linkedSignal, signal, untracked } from '@angular/core';
import { ExtractPreview, RecordView } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { Condition, EXTRACT_FIELD_TYPES, ExtractFieldType, ProfileModel } from '../core/profile.model';
import { EMPTY_QUERY, WorkspaceStore } from '../core/workspace.store';
import { ConditionsEditor } from '../profile/conditions-editor';
import { ProfileEdits } from '../profile/profile-edits';
import { HelpHint } from '../ui/help-hint';
import { DIALOG_REF } from '../ui/dialog.service';
import { DIALOG_FORM_STYLES } from './dialog-styles';
import { proposeExtraction } from './regex-builder';
import { addChartRule, addExtractRule, chartForField, extractRule, recordFieldText } from './rule-builders';

const PREVIEW_DELAY_MS = 300;

/** "Extract value": turns the selected part of a message into a typed field (with an optional chart). */
@Component({
  selector: 'imt-extract-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ConditionsEditor, HelpHint],
  template: `
    <p>Make the selected value a field you can filter, group by and chart. This adds an <span class="mono">[[extract]]</span> rule to the profile. <imt-help label="Extract" text="The regex is built from the text around your selection; the highlighted part becomes the named group. Edit it if it matches too little or too much." /></p>
    <div class="status">Message</div>
    <div class="box" data-testid="extract-message">{{ record().message }}</div>
    <label class="lbl">Regex (the named group is the value)<input class="input mono" [value]="regex()" (input)="regex.set($any($event.target).value)" /></label>
    <div class="row">
      <label class="lbl">Field name<input class="input mono" [value]="name()" (input)="name.set($any($event.target).value)" /></label>
      <label class="lbl">Type
        <select class="select" [value]="type()" (change)="type.set($any($event.target).value)">
          @for (t of types; track t) { <option [value]="t" [selected]="t === type()">{{ t }}</option> }
        </select>
      </label>
      @if (type() === 'bytes' || type() === 'duration') {
        <label class="lbl">Written in<input class="input" [value]="unit()" (input)="unit.set($any($event.target).value)" /></label>
        <label class="lbl">Store as<input class="input" [value]="to()" (input)="to.set($any($event.target).value)" /></label>
      } @else if (type() === 'number') {
        <label class="lbl">Unit<input class="input" [value]="unit()" (input)="unit.set($any($event.target).value)" /></label>
      }
    </div>
    <div class="status">Only for events where</div>
    <imt-conditions-editor [conditions]="when()" emptyText="Applies to every event." (changed)="when.set($event)" />
    <div class="status" role="status" aria-live="polite" data-testid="extract-preview">
      @if (preview(); as p) {
        @if (p.error) { <span class="err">{{ p.error }}</span> }
        @else {
          <strong>{{ p.matches }}</strong> of {{ p.candidates }} candidate events match.
          @for (s of p.samples; track s.recordId) { <div class="mono">{{ valuesText(s.values) }}</div> }
        }
      } @else { Checking… }
    </div>
    <label class="check"><input type="checkbox" [checked]="chart()" (change)="chart.set($any($event.target).checked)" /> Chart this field (average over time, one line per instance)</label>
    @if (error(); as e) { <div class="err" role="alert">{{ e }}</div> }
    <div class="foot">
      <button type="button" class="btn ghost" (click)="ref.close()">Cancel</button>
      <button type="button" class="btn primary" [disabled]="!valid() || saving()" (click)="save()">{{ saving() ? 'Applying…' : 'Add extractor' }}</button>
    </div>
  `,
  styles: DIALOG_FORM_STYLES,
})
export class ExtractDialog {
  protected readonly ref = inject(DIALOG_REF);
  private readonly edits = inject(ProfileEdits);
  private readonly backend = inject(ImtBackend);
  private readonly store = inject(WorkspaceStore);

  readonly record = input.required<RecordView>();
  /** Selected range in `record().message` (UTF-16 offsets). */
  readonly start = input.required<number>();
  readonly end = input.required<number>();

  protected readonly types: ExtractFieldType[] = EXTRACT_FIELD_TYPES;
  private readonly proposal = computed(() => proposeExtraction(this.record().message, this.start(), this.end()));
  protected readonly regex = linkedSignal(() => this.proposal().regex);
  protected readonly name = linkedSignal(() => this.proposal().name);
  protected readonly type = linkedSignal<ExtractFieldType>(() => this.proposal().type);
  protected readonly unit = linkedSignal(() => this.proposal().unit ?? '');
  protected readonly to = linkedSignal(() => this.proposal().to ?? '');
  protected readonly when = linkedSignal<Condition[]>(() => {
    const context = recordFieldText(this.record(), 'SourceContext');
    return context ? [{ field: 'SourceContext', op: 'eq', value: context }] : [];
  });
  protected readonly chart = signal(false);
  protected readonly saving = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly preview = signal<ExtractPreview | null>(null);
  protected readonly valid = computed(() => /^[A-Za-z_][A-Za-z0-9_]*$/.test(this.name()) && this.regex().includes(`(?P<${this.name()}>`));

  private model: ProfileModel | null = null;
  private readonly loaded = signal(false);
  private seq = 0;

  constructor() {
    void this.edits.current().then(
      ({ model }) => {
        this.model = model;
        this.loaded.set(true);
      },
      (e) => this.error.set(String(e)),
    );
    effect((onCleanup) => {
      if (!this.loaded()) return;
      const draft = this.draft();
      const mine = ++this.seq;
      untracked(() => this.preview.set(null));
      const timer = setTimeout(async () => {
        try {
          const { toml, rule } = this.apply(draft);
          const result = await this.backend.previewExtractor({ toml, extractor: rule.name, query: EMPTY_QUERY, limit: 3 });
          if (mine === this.seq) this.preview.set(result);
        } catch (e) {
          if (mine === this.seq) this.preview.set({ candidates: 0, matches: 0, failed: 0, samples: [], error: String(e) });
        }
      }, PREVIEW_DELAY_MS);
      onCleanup(() => clearTimeout(timer));
    });
  }

  private draft() {
    return { name: this.name(), field: this.name(), regex: this.regex(), type: this.type(), unit: this.unit() || undefined, to: this.to() || undefined, when: this.when() };
  }

  /** The current profile with the rule added (a copy), as TOML, plus the rule as named in it. */
  private apply(draft: ReturnType<ExtractDialog['draft']>) {
    const model = structuredClone(this.model ?? {});
    addExtractRule(model, extractRule(draft));
    const rule = model.extract![model.extract!.length - 1];
    return { toml: this.edits.toToml(model), rule };
  }

  protected valuesText(values: Record<string, unknown>): string {
    return Object.entries(values)
      .map(([k, v]) => `${k} = ${String(v)}`)
      .join(', ');
  }

  protected async save(): Promise<void> {
    this.saving.set(true);
    this.error.set(null);
    const draft = this.draft();
    try {
      await this.edits.apply((m) => {
        addExtractRule(m, extractRule(draft));
        if (this.chart()) addChartRule(m, chartForField(draft.field, this.store.effectiveGroupBy()));
      });
      this.store.setNotice(`Added the "${draft.field}" field; files were re-imported.`);
      this.ref.close(true);
    } catch (e) {
      this.error.set(e instanceof Error ? e.message : String(e));
    } finally {
      this.saving.set(false);
    }
  }
}
