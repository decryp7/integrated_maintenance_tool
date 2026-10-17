import { ChangeDetectionStrategy, Component, computed, inject, input, linkedSignal, signal } from '@angular/core';
import { Condition } from '../core/profile.model';
import { formatCount } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { ConditionsEditor } from '../profile/conditions-editor';
import { ProfileEdits } from '../profile/profile-edits';
import { DIALOG_REF } from '../ui/dialog.service';
import { DIALOG_FORM_STYLES } from './dialog-styles';
import { matchCount } from './match-count';
import { addPipelineStep, describeConditions, dropStep } from './rule-builders';

/** "Drop events like this": adds a `drop` step to the profile pipeline, with a live count of what it removes. */
@Component({
  selector: 'imt-drop-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ConditionsEditor],
  template: `
    <p>Remove every event matching all of these conditions the next time files are imported. This adds a <span class="mono">drop</span> step to the profile's pipeline.</p>
    <imt-conditions-editor [conditions]="conditions()" emptyText="Add a condition: a drop step without any would remove every event." (changed)="conditions.set($event)" />
    <div class="status" role="status">
      @if (count() === null) { Counting… } @else { This matches <strong>{{ fmt(count()!) }}</strong> events right now. }
    </div>
    <div class="status">In plain words: drop events where {{ summary() }}.</div>
    @if (error(); as e) { <div class="err" role="alert">{{ e }}</div> }
    <div class="foot">
      <button type="button" class="btn ghost" (click)="ref.close()">Cancel</button>
      <button type="button" class="btn primary" [disabled]="!conditions().length || saving()" (click)="save()">{{ saving() ? 'Applying…' : 'Add drop step' }}</button>
    </div>
  `,
  styles: DIALOG_FORM_STYLES,
})
export class DropDialog {
  protected readonly ref = inject(DIALOG_REF);
  private readonly edits = inject(ProfileEdits);
  private readonly store = inject(WorkspaceStore);

  /** Starting conditions (derived from the record the user clicked). */
  readonly initial = input<Condition[]>([]);
  protected readonly conditions = linkedSignal(() => this.initial());
  protected readonly saving = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly fmt = formatCount;
  protected readonly count = matchCount(() => this.conditions());
  protected readonly summary = computed(() => describeConditions(this.conditions()));

  protected async save(): Promise<void> {
    this.saving.set(true);
    this.error.set(null);
    try {
      await this.edits.apply((m) => addPipelineStep(m, dropStep(this.conditions())));
      this.store.setNotice('Added a drop step to the pipeline; files were re-imported.');
      this.ref.close(true);
    } catch (e) {
      this.error.set(e instanceof Error ? e.message : String(e));
    } finally {
      this.saving.set(false);
    }
  }
}
