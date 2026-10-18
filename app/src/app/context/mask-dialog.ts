import { ChangeDetectionStrategy, Component, computed, inject, input, linkedSignal, signal } from '@angular/core';
import { RecordView } from '../core/api.types';
import { formatCount } from '../core/format';
import { toJsRegex } from '../core/profile.model';
import { WorkspaceStore } from '../core/workspace.store';
import { ProfileEdits } from '../profile/profile-edits';
import { HelpHint } from '../ui/help-hint';
import { DIALOG_REF } from '../ui/dialog.service';
import { DIALOG_FORM_STYLES } from './dialog-styles';
import { matchCount } from './match-count';
import { addPipelineStep, maskStep } from './rule-builders';
import { maskRegexFor } from './regex-builder';

/** Applies a regex replacement to `text`; null when the regex is invalid. */
export function applyMask(text: string, regex: string, replace: string): string | null {
  try {
    return text.replace(toJsRegex(regex, 'g'), replace);
  } catch {
    return null;
  }
}

/** "Mask in message": hides the selected text (digits generalised) in every message via a `mask` pipeline step. */
@Component({
  selector: 'imt-mask-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [HelpHint],
  template: `
    <p><ng-container i18n="@@context.maskDialog.intro">Replace the selected text in every event's message, so secrets and ids never show up in the workspace. This adds a <span class="mono">mask</span> step to the pipeline.</ng-container>&ngsp;<imt-help label="Mask" i18n-label="@@context.maskDialog.helpLabel" i18n-text="@@context.maskDialog.helpText" text="The regex is generated from your selection: numbers become \\d+ so other ids match too. Use $1 in the replacement to keep a captured group." /></p>
    <div class="row">
      <label class="lbl"><ng-container i18n="@@context.maskDialog.regex">Regex</ng-container><input class="input mono" [value]="regex()" (input)="regex.set($any($event.target).value)" /></label>
      <label class="lbl"><ng-container i18n="@@context.maskDialog.replaceWith">Replace with</ng-container><input class="input mono" [value]="replace()" (input)="replace.set($any($event.target).value)" /></label>
    </div>
    <div class="status" i18n="@@context.maskDialog.before">Before</div>
    <div class="box" data-testid="mask-before">{{ record().message }}</div>
    <div class="status" i18n="@@context.maskDialog.after">After</div>
    @if (after() === null) {
      <div class="err" role="alert" i18n="@@context.maskDialog.invalid">This is not a valid regular expression.</div>
    } @else {
      <div class="box" data-testid="mask-after">{{ after() }}</div>
    }
    <div class="status" role="status">
      @if (count() === null) { <ng-container i18n="@@context.counting">Counting…</ng-container> } @else { <ng-container i18n="@@context.maskDialog.changes">Changes the message of <strong>{{ fmt(count()!) }}</strong> events.</ng-container> }
    </div>
    @if (error(); as e) { <div class="err" role="alert">{{ e }}</div> }
    <div class="foot">
      <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@context.cancel">Cancel</button>
      <button type="button" class="btn primary" [disabled]="after() === null || !regex() || saving()" (click)="save()">{{ saving() ? applying : addStep }}</button>
    </div>
  `,
  styles: DIALOG_FORM_STYLES,
})
export class MaskDialog {
  protected readonly ref = inject(DIALOG_REF);
  private readonly edits = inject(ProfileEdits);
  private readonly store = inject(WorkspaceStore);

  readonly record = input.required<RecordView>();
  readonly selected = input.required<string>();
  protected readonly regex = linkedSignal(() => maskRegexFor(this.selected()));
  protected readonly replace = signal('***');
  protected readonly saving = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly fmt = formatCount;
  protected readonly applying = $localize`:@@context.applying:Applying…`;
  protected readonly addStep = $localize`:@@context.maskDialog.addStep:Add mask step`;
  protected readonly after = computed(() => applyMask(this.record().message, this.regex(), this.replace()));
  protected readonly count = matchCount(() => [{ field: '@message', op: 'matches', value: this.regex() }]);

  protected async save(): Promise<void> {
    this.saving.set(true);
    this.error.set(null);
    try {
      await this.edits.apply((m) => addPipelineStep(m, maskStep(this.regex(), this.replace())));
      this.store.setNotice($localize`:@@context.maskDialog.added:Added a mask step to the pipeline; files were re-imported.`);
      this.ref.close(true);
    } catch (e) {
      this.error.set(e instanceof Error ? e.message : String(e));
    } finally {
      this.saving.set(false);
    }
  }
}
