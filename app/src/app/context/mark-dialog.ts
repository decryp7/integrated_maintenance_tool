import { ChangeDetectionStrategy, Component, computed, inject, input, linkedSignal, signal } from '@angular/core';
import { Condition } from '../core/profile.model';
import { formatCount } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { ConditionsEditor } from '../profile/conditions-editor';
import { ProfileEdits } from '../profile/profile-edits';
import { DIALOG_REF } from '../ui/dialog.service';
import { DIALOG_FORM_STYLES } from './dialog-styles';
import { matchCount } from './match-count';
import { addMarkerRule, markerRule, slug } from './rule-builders';

/** "Mark events like this": adds a `[[marker]]` rule so matching events get a timeline marker. */
@Component({
  selector: 'imt-mark-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ConditionsEditor],
  template: `
    <p>Show matching events as markers on the timeline and in the grid, and list them under "Marked events only".</p>
    <div class="row">
      <label class="lbl">Label<input class="input" [value]="label()" (input)="label.set($any($event.target).value)" /></label>
      <label class="lbl">Kind (id)<input class="input mono" [value]="kind()" (input)="kindEdited.set($any($event.target).value)" /></label>
      <label class="lbl">Colour<input class="input color" type="color" [value]="color()" (input)="color.set($any($event.target).value)" /></label>
    </div>
    <label class="check"><input type="checkbox" [checked]="lifecycle()" (change)="lifecycle.set($any($event.target).checked)" /> Lifecycle marker (start/stop: stays visible even when filters hide it)</label>
    <imt-conditions-editor [conditions]="conditions()" emptyText="Add a condition: a marker without any would mark every event." (changed)="conditions.set($event)" />
    <div class="status" role="status">
      @if (count() === null) { Counting… } @else { This marks <strong>{{ fmt(count()!) }}</strong> events right now. }
    </div>
    @if (error(); as e) { <div class="err" role="alert">{{ e }}</div> }
    <div class="foot">
      <button type="button" class="btn ghost" (click)="ref.close()">Cancel</button>
      <button type="button" class="btn primary" [disabled]="!conditions().length || !kind() || saving()" (click)="save()">{{ saving() ? 'Applying…' : 'Add marker' }}</button>
    </div>
  `,
  styles: [DIALOG_FORM_STYLES, '.color { width: 56px; padding: 2px; }'],
})
export class MarkDialog {
  protected readonly ref = inject(DIALOG_REF);
  private readonly edits = inject(ProfileEdits);
  private readonly store = inject(WorkspaceStore);

  readonly initial = input<Condition[]>([]);
  readonly suggestedLabel = input('Marked events');
  protected readonly conditions = linkedSignal(() => this.initial());
  protected readonly label = linkedSignal(() => this.suggestedLabel());
  protected readonly kindEdited = signal<string | null>(null);
  protected readonly kind = computed(() => this.kindEdited() ?? slug(this.label()));
  protected readonly color = signal('#e9ae3f');
  protected readonly lifecycle = signal(false);
  protected readonly saving = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly fmt = formatCount;
  protected readonly count = matchCount(() => this.conditions());

  protected async save(): Promise<void> {
    this.saving.set(true);
    this.error.set(null);
    try {
      const rule = markerRule({ kind: this.kind(), label: this.label(), color: this.color(), lifecycle: this.lifecycle(), when: this.conditions() });
      await this.edits.apply((m) => addMarkerRule(m, rule));
      this.store.setNotice(`Added the "${rule.label}" marker; files were re-imported.`);
      this.ref.close(true);
    } catch (e) {
      this.error.set(e instanceof Error ? e.message : String(e));
    } finally {
      this.saving.set(false);
    }
  }
}
