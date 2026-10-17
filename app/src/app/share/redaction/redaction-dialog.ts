import { ChangeDetectionStrategy, Component, computed, effect, inject, signal, untracked } from '@angular/core';
import { RedactDetector, REDACT_DETECTORS, RedactionPreview, RedactMode } from '../../core/api.types';
import { ProfileModel } from '../../core/profile.model';
import { ProfileEdits } from '../../profile/profile-edits';
import { errorMessage } from '../../tools/tool-errors';
import { DIALOG_REF } from '../../ui/dialog.service';
import { DIALOG_FORM_STYLES } from '../../context/dialog-styles';
import { ShareBackend } from '../share-backend';
import { DETECTOR_DESCRIPTIONS, parseFieldList, readRedact, RedactForm, RedactRuleForm, writeRedact } from './redact-section';

const SAMPLE = 'Login by alice@contoso.com from 10.0.0.12 with password=hunter2; Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload12.sig';
const PSEUDO = ['@message', '@exception', '@template'] as const;
const PREVIEW_MS = 250;

/** Edits the profile's `[redact]` section with a live preview computed by the engine. */
@Component({
  selector: 'imt-redaction-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (!form()) {
      <p role="status">Loading the profile…</p>
    } @else {
      <fieldset class="group">
        <legend>Detectors</legend>
        <div class="grid">
          @for (d of detectors; track d) {
            <label class="check" [title]="desc[d]">
              <input type="checkbox" [checked]="has(d)" (change)="toggleDetector(d, $any($event.target).checked)" />
              <span class="mono">{{ d }}</span>
              <span class="desc">{{ desc[d] }}</span>
            </label>
          }
        </div>
      </fieldset>

      <div class="row">
        <label class="lbl">Replace values by
          <select class="select" [value]="form()!.mode" (change)="patch({ mode: $any($event.target).value })" aria-label="Replace values by">
            <option value="mask">mask (&lt;email&gt;)</option>
            <option value="hash">hash (&lt;email:1a2b…&gt;, equal values match within an export)</option>
            <option value="drop">drop (removed)</option>
          </select>
        </label>
      </div>

      <fieldset class="group">
        <legend>Where</legend>
        <div class="inline">
          <label class="check"><input type="checkbox" [checked]="allFields()" (change)="setAll($any($event.target).checked)" /> All fields (*)</label>
          @for (p of pseudo; track p) {
            <label class="check"><input type="checkbox" [disabled]="allFields()" [checked]="allFields() || form()!.fields.includes(p)" (change)="togglePseudo(p, $any($event.target).checked)" /> <span class="mono">{{ p }}</span></label>
          }
        </div>
        <label class="lbl">Other fields (comma-separated)
          <input class="input" [disabled]="allFields()" [value]="otherFields()" (change)="setOther($any($event.target).value)" placeholder="User, ClientIp" />
        </label>
      </fieldset>

      <label class="check"><input type="checkbox" [checked]="form()!.onExport" (change)="patch({ onExport: $any($event.target).checked })" /> Redact exports, reports and bundles by default</label>

      <fieldset class="group">
        <legend>Custom rules</legend>
        @for (r of form()!.rules; track $index; let i = $index) {
          <div class="rule">
            <label class="lbl">Name <input class="input" [value]="r.name" (input)="setRule(i, { name: $any($event.target).value })" /></label>
            <label class="lbl grow">Regex (a <span class="mono">value</span> group is the part replaced) <input class="input mono" [value]="r.regex" (input)="setRule(i, { regex: $any($event.target).value })" /></label>
            <label class="lbl">Replace (optional) <input class="input mono" [value]="r.replace" (input)="setRule(i, { replace: $any($event.target).value })" /></label>
            <button type="button" class="btn ghost" [attr.aria-label]="'Remove rule ' + (r.name || i + 1)" (click)="removeRule(i)">Remove</button>
          </div>
        }
        <div><button type="button" class="btn" (click)="addRule()">Add rule</button></div>
      </fieldset>

      <label class="lbl">Sample text
        <textarea class="input mono" rows="2" [value]="sample()" (input)="sample.set($any($event.target).value)" aria-label="Sample text"></textarea>
      </label>
      <div class="preview" aria-live="polite">
        @if (preview(); as p) {
          @if (p.error) {
            <p class="err" role="alert">{{ p.error }}</p>
          } @else {
            <div class="box" data-testid="redaction-preview">{{ p.after }}</div>
            <p class="status">
              @for (h of p.hits; track h.rule; let last = $last) {
                <span class="mono">{{ h.rule }}</span> {{ h.count }}{{ last ? '' : ' · ' }}
              }
            </p>
          }
        } @else {
          <p class="status">Previewing…</p>
        }
      </div>
    }
    @if (error(); as e) { <p class="err" role="alert">{{ e }}</p> }
    <div class="foot">
      <button class="btn" type="button" (click)="ref.close()">Cancel</button>
      <button class="btn primary" type="button" [disabled]="!form() || busy() || !!preview()?.error" (click)="save()">{{ busy() ? 'Saving…' : 'Save' }}</button>
    </div>
  `,
  styles: `
    ${DIALOG_FORM_STYLES}
    .group { border: 1px solid var(--border); border-radius: 6px; padding: 8px 10px; margin: 0; display: flex; flex-direction: column; gap: 6px; min-width: 0; }
    legend { font-size: 11px; color: var(--muted); padding: 0 4px; }
    .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: 4px 12px; }
    .check { font-size: 12px; }
    .desc { color: var(--muted); font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .inline { display: flex; gap: 14px; flex-wrap: wrap; }
    .rule { display: flex; gap: 8px; align-items: flex-end; flex-wrap: wrap; }
    .rule .grow { flex: 2; }
  `,
})
export class RedactionDialog {
  protected readonly ref = inject(DIALOG_REF);
  private readonly edits = inject(ProfileEdits);
  private readonly share = inject(ShareBackend);

  protected readonly detectors = REDACT_DETECTORS;
  protected readonly desc = DETECTOR_DESCRIPTIONS;
  protected readonly pseudo = PSEUDO;

  private model: ProfileModel | null = null;
  protected readonly form = signal<RedactForm | null>(null);
  protected readonly sample = signal(SAMPLE);
  protected readonly preview = signal<RedactionPreview | null>(null);
  protected readonly busy = signal(false);
  protected readonly error = signal<string | null>(null);

  protected readonly allFields = computed(() => !!this.form()?.fields.includes('*'));
  protected readonly otherFields = computed(() => (this.form()?.fields ?? []).filter((f) => f !== '*' && !(PSEUDO as readonly string[]).includes(f)).join(', '));

  private timer: ReturnType<typeof setTimeout> | undefined;
  private seq = 0;

  constructor() {
    void this.load();
    effect(() => {
      const form = this.form();
      const sample = this.sample();
      if (!form) return;
      untracked(() => this.schedule(form, sample));
    });
  }

  private async load(): Promise<void> {
    try {
      const { model } = await this.edits.current();
      this.model = model;
      this.form.set(readRedact(model));
    } catch (e) {
      this.error.set(`Could not read the profile: ${errorMessage(e)}`);
    }
  }

  private candidate(form: RedactForm): string {
    const model = structuredClone(this.model ?? {}) as ProfileModel;
    writeRedact(model, form);
    return this.edits.toToml(model);
  }

  private schedule(form: RedactForm, sample: string): void {
    clearTimeout(this.timer);
    const mine = ++this.seq;
    this.timer = setTimeout(() => void this.runPreview(mine, form, sample), PREVIEW_MS);
  }

  private async runPreview(mine: number, form: RedactForm, sample: string): Promise<void> {
    try {
      const p = await this.share.previewRedaction({ sample, toml: this.candidate(form) });
      if (mine === this.seq) this.preview.set(p);
    } catch (e) {
      if (mine === this.seq) this.preview.set({ before: sample, after: sample, hits: [], error: errorMessage(e) });
    }
  }

  protected has(d: RedactDetector): boolean {
    return !!this.form()?.detectors.includes(d);
  }

  protected patch(p: Partial<RedactForm>): void {
    this.form.update((f) => (f ? { ...f, ...p } : f));
  }

  protected toggleDetector(d: RedactDetector, on: boolean): void {
    const f = this.form();
    if (f) this.patch({ detectors: on ? [...f.detectors.filter((x) => x !== d), d] : f.detectors.filter((x) => x !== d) });
  }

  protected setAll(on: boolean): void {
    const f = this.form();
    if (f) this.patch({ fields: on ? ['*'] : ['@message', '@exception'] });
  }

  protected togglePseudo(p: string, on: boolean): void {
    const f = this.form();
    if (f) this.patch({ fields: on ? [...f.fields.filter((x) => x !== p), p] : f.fields.filter((x) => x !== p) });
  }

  protected setOther(text: string): void {
    const f = this.form();
    if (f) this.patch({ fields: [...f.fields.filter((x) => (PSEUDO as readonly string[]).includes(x)), ...parseFieldList(text)] });
  }

  protected addRule(): void {
    const f = this.form();
    if (f) this.patch({ rules: [...f.rules, { name: `rule-${f.rules.length + 1}`, regex: '', replace: '' }] });
  }

  protected setRule(i: number, p: Partial<RedactRuleForm>): void {
    const f = this.form();
    if (f) this.patch({ rules: f.rules.map((r, j) => (j === i ? { ...r, ...p } : r)) });
  }

  protected removeRule(i: number): void {
    const f = this.form();
    if (f) this.patch({ rules: f.rules.filter((_, j) => j !== i) });
  }

  protected async save(): Promise<void> {
    const form = this.form();
    if (!form) return;
    this.busy.set(true);
    this.error.set(null);
    try {
      await this.edits.apply((m) => writeRedact(m, form));
      this.ref.close();
    } catch (e) {
      this.error.set(errorMessage(e));
    } finally {
      this.busy.set(false);
    }
  }
}
