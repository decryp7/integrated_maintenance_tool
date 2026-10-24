import { formatCount } from '../../shared/format/units';
import { ChangeDetectionStrategy, Component, computed, DestroyRef, effect, inject, signal, untracked } from '@angular/core';
import { RedactDetector, REDACT_DETECTORS, RedactionPreview } from '../../core/api.types';
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
  selector: 'debrief-redaction-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (!form()) {
      <p role="status" i18n="@@share.redactionDialog.loading">Loading the profile…</p>
    } @else {
      <fieldset class="group">
        <legend i18n="@@share.redactionDialog.detectors">Detectors</legend>
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
        <label class="lbl"><ng-container i18n="@@share.redactionDialog.replaceBy">Replace values by</ng-container>
          <select class="select" [value]="form()!.mode" (change)="patch({ mode: $any($event.target).value })" aria-label="Replace values by" i18n-aria-label="@@share.redactionDialog.replaceByAria">
            <option value="mask" i18n="@@share.redactionDialog.modeMask">mask (&lt;email&gt;)</option>
            <option value="hash" i18n="@@share.redactionDialog.modeHash">hash (&lt;email:1a2b…&gt;, equal values match within an export)</option>
            <option value="drop" i18n="@@share.redactionDialog.modeDrop">drop (removed)</option>
          </select>
        </label>
      </div>

      <fieldset class="group">
        <legend i18n="@@share.redactionDialog.where">Where</legend>
        <div class="inline">
          <label class="check"><input type="checkbox" [checked]="allFields()" (change)="setAll($any($event.target).checked)" />&ngsp;<ng-container i18n="@@share.redactionDialog.allFields">All fields (*)</ng-container></label>
          @for (p of pseudo; track p) {
            <label class="check"><input type="checkbox" [disabled]="allFields()" [checked]="allFields() || form()!.fields.includes(p)" (change)="togglePseudo(p, $any($event.target).checked)" /> <span class="mono">{{ p }}</span></label>
          }
        </div>
        <label class="lbl"><ng-container i18n="@@share.redactionDialog.otherFields">Other fields (comma-separated)</ng-container>
          <input class="input" [disabled]="allFields()" [value]="otherFields()" (change)="setOther($any($event.target).value)" placeholder="User, ClientIp" />
        </label>
      </fieldset>

      <label class="check"><input type="checkbox" [checked]="form()!.onExport" (change)="patch({ onExport: $any($event.target).checked })" />&ngsp;<ng-container i18n="@@share.redactionDialog.onExport">Redact exports, reports and bundles by default</ng-container></label>

      <fieldset class="group">
        <legend i18n="@@share.redactionDialog.customRules">Custom rules</legend>
        @for (r of form()!.rules; track $index; let i = $index) {
          <div class="rule">
            <label class="lbl"><ng-container i18n="@@share.redactionDialog.ruleName">Name</ng-container>&ngsp;<input class="input" [value]="r.name" (input)="setRule(i, { name: $any($event.target).value })" /></label>
            <label class="lbl grow"><ng-container i18n="@@share.redactionDialog.ruleRegex">Regex (a <span class="mono">value</span> group is the part replaced)</ng-container>&ngsp;<input class="input mono" [value]="r.regex" (input)="setRule(i, { regex: $any($event.target).value })" /></label>
            <label class="lbl"><ng-container i18n="@@share.redactionDialog.ruleReplace">Replace (optional)</ng-container>&ngsp;<input class="input mono" [value]="r.replace" (input)="setRule(i, { replace: $any($event.target).value })" /></label>
            <button type="button" class="btn ghost" [attr.aria-label]="removeLabel(r.name, i)" (click)="removeRule(i)" i18n="@@share.redactionDialog.removeRule">Remove</button>
          </div>
        }
        <div><button type="button" class="btn" (click)="addRule()" i18n="@@share.redactionDialog.addRule">Add rule</button></div>
      </fieldset>

      <label class="lbl"><ng-container i18n="@@share.redactionDialog.sample">Sample text</ng-container>
        <textarea class="input mono" rows="2" [value]="sample()" (input)="sample.set($any($event.target).value)" aria-label="Sample text" i18n-aria-label="@@share.redactionDialog.sampleAria"></textarea>
      </label>
      <div class="preview" aria-live="polite">
        @if (preview(); as p) {
          @if (p.error) {
            <p class="err" role="alert">{{ p.error }}</p>
          } @else {
            <div class="box" data-testid="redaction-preview">{{ p.after }}</div>
            <p class="status">
              @for (h of p.hits; track h.rule; let last = $last) {
                <span class="mono">{{ h.rule }}</span> {{ fmt(h.count) }}{{ last ? '' : ' · ' }}
              }
            </p>
          }
        } @else {
          <p class="status" i18n="@@share.redactionDialog.previewing">Previewing…</p>
        }
      </div>
    }
    @if (error(); as e) { <p class="err" role="alert">{{ e }}</p> }
    <div class="foot">
      <button class="btn" type="button" (click)="ref.close()" i18n="@@share.redactionDialog.cancel">Cancel</button>
      <button class="btn primary" type="button" [disabled]="!form() || busy() || !!preview()?.error" (click)="save()">{{ saveLabel() }}</button>
    </div>
  `,
  styles: `
    ${DIALOG_FORM_STYLES}
    .group { border: 1px solid var(--border); border-radius: 6px; padding: 8px 10px; margin: 0; display: flex; flex-direction: column; gap: 6px; min-width: 0; }
    legend { font-size: var(--fs-xs); color: var(--muted); padding: 0 4px; }
    .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: 4px 12px; }
    .check { font-size: var(--fs-sm); }
    .desc { color: var(--muted); font-size: var(--fs-xs); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .inline { display: flex; gap: 14px; flex-wrap: wrap; }
    .rule { display: flex; gap: 8px; align-items: flex-end; flex-wrap: wrap; }
    .rule .grow { flex: 2; }
  `,
})
export class RedactionDialog {
  static readonly dialogId = 'redaction';
  protected readonly fmt = formatCount;
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

  protected readonly saveLabel = computed(() => (this.busy() ? $localize`:@@share.redactionDialog.saving:Saving…` : $localize`:@@share.redactionDialog.save:Save`));
  protected readonly allFields = computed(() => !!this.form()?.fields.includes('*'));
  protected readonly otherFields = computed(() => (this.form()?.fields ?? []).filter((f) => f !== '*' && !(PSEUDO as readonly string[]).includes(f)).join(', '));

  private timer: ReturnType<typeof setTimeout> | undefined;
  private seq = 0;

  constructor() {
    // A closed dialog asks nothing more: its pending preview is cancelled.
    inject(DestroyRef).onDestroy(() => {
      clearTimeout(this.timer);
      this.seq++;
    });
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
      this.error.set($localize`:@@share.redactionDialog.readFailed:Could not read the profile: ${errorMessage(e)}:error:`);
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

  protected removeLabel(name: string, i: number): string {
    return $localize`:@@share.redactionDialog.removeRuleAria:Remove rule ${name || i + 1}:rule:`;
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
