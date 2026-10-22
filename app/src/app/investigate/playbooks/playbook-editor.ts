import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, inject, linkedSignal, signal } from '@angular/core';
import { Playbook, PlaybookAction, PlaybookExpect, PlaybookStep } from '../../core/api.types';
import { Icon } from '../../shared/icon';
import { ACTION_TYPES, actionLabel, defaultAction, defaultExpect, EXPECT_KINDS, expectLabel, missingField, moved, newStep } from './playbook-model';
import { PlaybookStore } from './playbook.store';

/**
 * Edits a playbook: title, description and steps (add, remove, reorder), each step with its action and the action's
 * fields. Required fields are checked early for convenience; the engine validates on save and its message is shown.
 */
@Component({
  selector: 'imt-playbook-editor',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, NgTemplateOutlet],
  template: `
    <form class="editor" data-testid="playbook-editor" (submit)="$event.preventDefault(); save()">
      <h3 class="head" i18n="@@playbooks.editor.heading">Edit playbook</h3>
      <label class="field">
        <span i18n="@@playbooks.editor.title">Title</span>
        <input type="text" data-testid="pb-title" [value]="draft().title" (input)="patch({ title: val($event) })" />
      </label>
      @if (store.editingIsNew()) {
        <label class="field">
          <span i18n="@@playbooks.editor.id">Id (lowercase letters, digits and dashes)</span>
          <input type="text" data-testid="pb-id" [value]="draft().id" (input)="patch({ id: val($event) })" />
        </label>
      }
      <label class="field">
        <span i18n="@@playbooks.editor.description">Description</span>
        <textarea rows="2" data-testid="pb-description" [value]="draft().description" (input)="patch({ description: val($event) })"></textarea>
      </label>

      <ol class="steps">
        @for (s of draft().steps; track s.id; let i = $index; let last = $last) {
          <li class="step" [attr.data-testid]="'pb-step-' + i">
            <div class="step-head">
              <strong i18n="@@playbooks.editor.stepN">Step {{ i + 1 }}</strong>
              <span class="spacer"></span>
              <button type="button" class="icon-btn" aria-label="Move step up" i18n-aria-label="@@playbooks.editor.up" [disabled]="i === 0" (click)="move(i, -1)">
                <span aria-hidden="true">↑</span>
              </button>
              <button type="button" class="icon-btn" aria-label="Move step down" i18n-aria-label="@@playbooks.editor.down" [disabled]="last" (click)="move(i, 1)">
                <span aria-hidden="true">↓</span>
              </button>
              <button type="button" class="icon-btn" aria-label="Remove step" i18n-aria-label="@@playbooks.editor.remove" (click)="removeStep(i)"><imt-icon name="trash" [size]="12" /></button>
            </div>
            <label class="field">
              <span i18n="@@playbooks.editor.stepTitle">Step title</span>
              <input type="text" [attr.data-testid]="'pb-step-title-' + i" [value]="s.title" (input)="patchStep(i, { title: val($event) })" />
            </label>
            <label class="field">
              <span i18n="@@playbooks.editor.instruction">Instruction</span>
              <textarea rows="2" [value]="s.instruction" (input)="patchStep(i, { instruction: val($event) })"></textarea>
            </label>
            <label class="field">
              <span i18n="@@playbooks.editor.action">Action</span>
              <select [attr.data-testid]="'pb-step-action-' + i" [value]="s.action.type" (change)="setActionType(i, val($event))">
                @for (t of actionTypes; track t) {
                  <option [value]="t" [selected]="t === s.action.type">{{ label(t) }}</option>
                }
              </select>
            </label>
            @switch (s.action.type) {
              @case ('query') {
                <ng-container *ngTemplateOutlet="ql; context: { i: i, ql: qlOf(s.action) }" />
              }
              @case ('filter') {
                <ng-container *ngTemplateOutlet="ql; context: { i: i, ql: qlOf(s.action) }" />
              }
              @case ('check') {
                <ng-container *ngTemplateOutlet="ql; context: { i: i, ql: qlOf(s.action) }" />
                <label class="field">
                  <span i18n="@@playbooks.editor.expect">Expected outcome</span>
                  <select [attr.data-testid]="'pb-step-expect-' + i" [value]="expectOf(s.action).kind" (change)="setExpectKind(i, val($event))">
                    @for (kind of expectKinds; track kind) {
                      <option [value]="kind" [selected]="kind === expectOf(s.action).kind">{{ expectText(kind) }}</option>
                    }
                  </select>
                </label>
                @if (needsN(s.action)) {
                  <label class="field">
                    <span i18n="@@playbooks.editor.n">Number of events (n)</span>
                    <input type="number" min="0" [attr.data-testid]="'pb-step-n-' + i" [value]="nOf(s.action)" (input)="setN(i, val($event))" />
                  </label>
                }
              }
              @case ('openView') {
                <label class="field">
                  <span i18n="@@playbooks.editor.view">View id (e.g. problems)</span>
                  <input type="text" [attr.data-testid]="'pb-step-view-' + i" [value]="viewOf(s.action)" (input)="setView(i, val($event))" />
                </label>
              }
            }
          </li>
        }
      </ol>
      <ng-template #ql let-i="i" let-text="ql">
        <label class="field">
          <span i18n="@@playbooks.editor.ql">Query</span>
          <textarea rows="2" class="mono" [attr.data-testid]="'pb-step-ql-' + i" [value]="text" (input)="setQl(i, val($event))"></textarea>
        </label>
      </ng-template>

      <button type="button" class="btn" data-testid="pb-add-step" (click)="addStep()"><imt-icon name="plus" [size]="12" /> <span i18n="@@playbooks.editor.addStep">Add step</span></button>

      @if (problem(); as p) {
        <div class="error" role="alert" data-testid="pb-editor-problem">{{ p }}</div>
      }
      @if (store.actionError(); as e) {
        <div class="error" role="alert" data-testid="pb-save-error">{{ e }}</div>
      }
      <div class="buttons">
        <button type="submit" class="btn primary" data-testid="pb-save" i18n="@@playbooks.editor.save">Save</button>
        <button type="button" class="btn" data-testid="pb-cancel" (click)="store.cancelEdit()" i18n="@@playbooks.editor.cancel">Cancel</button>
      </div>
    </form>
  `,
  styles: `
    :host { display: block; }
    .editor { display: flex; flex-direction: column; gap: 8px; }
    .head { margin: 0; font-size: var(--fs-md); }
    .field { display: flex; flex-direction: column; gap: 2px; font-size: var(--fs-sm); color: var(--text-2); }
    .field input, .field textarea, .field select { font: inherit; color: var(--text); background: var(--bg-0); border: 1px solid var(--border-2); border-radius: 4px; padding: 3px 6px; }
    .field input:focus-visible, .field textarea:focus-visible, .field select:focus-visible { outline: 2px solid var(--focus-ring); }
    .mono { font-family: var(--font-mono, monospace); }
    .steps { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
    .step { display: flex; flex-direction: column; gap: 6px; padding: 8px 10px; border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); }
    .step-head { display: flex; align-items: center; gap: 4px; }
    .spacer { flex: 1; }
    .error { color: var(--danger-text); font-size: var(--fs-sm); }
    .buttons { display: flex; gap: 8px; }
  `,
})
export class PlaybookEditor {
  protected readonly store = inject(PlaybookStore);
  /** The copy being edited; follows the store's `editing` when it is replaced. */
  protected readonly draft = linkedSignal<Playbook>(() => this.store.editing() ?? { version: 1, id: '', title: '', description: '', tags: [], steps: [] });
  protected readonly checked = signal(false);
  protected readonly actionTypes = ACTION_TYPES;
  protected readonly expectKinds = EXPECT_KINDS;

  protected readonly problem = () => (this.checked() ? missingField(this.draft()) : null);

  protected label = actionLabel;
  protected expectText = expectLabel;

  protected val(e: Event): string {
    return (e.target as HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement).value;
  }
  protected qlOf(a: PlaybookAction): string {
    return a.type === 'query' || a.type === 'filter' || a.type === 'check' ? a.ql : '';
  }
  protected viewOf(a: PlaybookAction): string {
    return a.type === 'openView' ? a.view : '';
  }
  protected expectOf(a: PlaybookAction): PlaybookExpect {
    return a.type === 'check' ? a.expect : { kind: 'noMatch' };
  }
  protected needsN(a: PlaybookAction): boolean {
    const k = this.expectOf(a).kind;
    return k === 'atLeast' || k === 'atMost';
  }
  protected nOf(a: PlaybookAction): number {
    const e = this.expectOf(a);
    return e.kind === 'atLeast' || e.kind === 'atMost' ? e.n : 0;
  }

  protected patch(change: Partial<Playbook>): void {
    this.draft.update((d) => ({ ...d, ...change }));
  }
  protected patchStep(i: number, change: Partial<PlaybookStep>): void {
    this.draft.update((d) => ({ ...d, steps: d.steps.map((s, j) => (j === i ? { ...s, ...change } : s)) }));
  }
  protected setActionType(i: number, type: string): void {
    this.patchStep(i, { action: defaultAction(type as PlaybookAction['type']) });
  }
  protected setQl(i: number, ql: string): void {
    const a = this.draft().steps[i]?.action;
    if (a && (a.type === 'query' || a.type === 'filter' || a.type === 'check')) this.patchStep(i, { action: { ...a, ql } });
  }
  protected setView(i: number, view: string): void {
    this.patchStep(i, { action: { type: 'openView', view } });
  }
  protected setExpectKind(i: number, kind: string): void {
    const a = this.draft().steps[i]?.action;
    if (a?.type === 'check') this.patchStep(i, { action: { ...a, expect: defaultExpect(kind as PlaybookExpect['kind'], Math.max(1, this.nOf(a))) } });
  }
  protected setN(i: number, raw: string): void {
    const a = this.draft().steps[i]?.action;
    const n = Math.max(0, Math.floor(Number(raw)) || 0);
    if (a?.type === 'check' && (a.expect.kind === 'atLeast' || a.expect.kind === 'atMost')) this.patchStep(i, { action: { ...a, expect: { kind: a.expect.kind, n } } });
  }
  protected addStep(): void {
    this.draft.update((d) => ({ ...d, steps: [...d.steps, newStep(d.steps)] }));
  }
  protected removeStep(i: number): void {
    this.draft.update((d) => ({ ...d, steps: d.steps.filter((_, j) => j !== i) }));
  }
  protected move(i: number, delta: number): void {
    this.draft.update((d) => ({ ...d, steps: moved(d.steps, i, delta) }));
  }

  protected async save(): Promise<void> {
    this.checked.set(true);
    if (missingField(this.draft())) return;
    await this.store.save(this.draft());
  }
}
