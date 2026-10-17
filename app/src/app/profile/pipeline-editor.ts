import { CdkDrag, CdkDragDrop, CdkDragHandle, CdkDropList } from '@angular/cdk/drag-drop';
import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { LEVELS } from '../core/api.types';
import { Condition, PipelineStep, PipelineStepKind } from '../core/profile.model';
import { WorkspaceStore } from '../core/workspace.store';
import { Icon } from '../shared/icon';
import { HelpHint } from '../ui/help-hint';
import { ConditionsEditor } from './conditions-editor';
import { ProfileDraft } from './profile-draft';
import { STEP_FIELDS, StepField } from './pipeline-fields';
import { defaultStep, duplicateStep, kindInfo, moveItem, STEP_CATALOG, STEP_GROUPS, stepStatus, summarizeStep, transformPlugins } from './pipeline-model';
import { StringList } from './string-list';

const HELP =
  'The pipeline runs on every event after it was parsed and before it is indexed, top to bottom. Use it to drop noise, hide secrets, fix levels or add fields. Steps can be reordered, duplicated and disabled.';

/** The ordered `[[pipeline]]` as an activity flow: a palette of step kinds and a vertical chain of cards. */
@Component({
  selector: 'imt-pipeline-editor',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, ConditionsEditor, StringList, HelpHint, CdkDropList, CdkDrag, CdkDragHandle],
  template: `
    <h2>Pipeline <imt-help label="Pipeline" [text]="help" /></h2>
    <p class="help">Each event flows through these steps in order after parsing. Add an activity, then fill in what it should do.</p>

    <div class="palette" role="group" aria-label="Add an activity">
      @for (g of groups; track g.name) {
        <div class="pal-group">
          <span class="section-title">{{ g.name }}</span>
          <div class="pal-items">
            @for (k of g.kinds; track k.kind) {
              <button class="btn" type="button" [title]="k.blurb" [disabled]="k.kind === 'plugin' && !plugins().length" (click)="add(k.kind)">
                <imt-icon [name]="k.icon" [size]="13" /> {{ k.label }}
              </button>
            }
          </div>
          @if (g.name === 'Extend' && !plugins().length) {
            <span class="faint small">No transform plugins are loaded.</span>
          }
        </div>
      }
    </div>

    <div class="flow" cdkDropList (cdkDropListDropped)="drop($event)" aria-label="Pipeline steps">
      <div class="end-node">Parsed events</div>
      @for (s of steps(); track $index; let i = $index) {
        <div class="arrow" aria-hidden="true">↓</div>
        <div class="card" [class.off]="s.disabled" cdkDrag cdkDragLockAxis="y" [attr.data-step]="s.step">
          <div class="card-head">
            <button class="icon-btn handle" type="button" cdkDragHandle [attr.aria-label]="'Drag step ' + (i + 1) + ' to reorder'" title="Drag to reorder"><imt-icon name="drag" [size]="14" [stroke]="3" /></button>
            <span class="num">{{ i + 1 }}</span>
            <imt-icon [name]="icon(s.step)" [size]="15" />
            <span class="summary">{{ summary(s) }}</span>
            <span class="stat" [class.ok]="status(s, i).state === 'ok'" [class.dim]="status(s, i).state !== 'ok'" [attr.data-state]="status(s, i).state">{{ status(s, i).text }}</span>
            <span class="actions">
              <button class="icon-btn" type="button" [attr.aria-label]="'Move step ' + (i + 1) + ' up'" title="Move up" [disabled]="i === 0" (click)="move(i, -1)">↑</button>
              <button class="icon-btn" type="button" [attr.aria-label]="'Move step ' + (i + 1) + ' down'" title="Move down" [disabled]="i === steps().length - 1" (click)="move(i, 1)">↓</button>
              <button class="icon-btn" type="button" [attr.aria-label]="'Duplicate step ' + (i + 1)" title="Duplicate" (click)="duplicate(i)"><imt-icon name="copy" [size]="13" /></button>
              <label class="off-toggle" [title]="s.disabled ? 'Enable this step' : 'Disable this step (kept in the profile)'">
                <input type="checkbox" [checked]="!!s.disabled" [attr.aria-label]="'Disable step ' + (i + 1)" (change)="setStep(i, 'disabled', $any($event.target).checked || undefined)" /> off
              </label>
              <button class="icon-btn" type="button" [attr.aria-label]="'Remove step ' + (i + 1)" title="Remove" (click)="remove(i)"><imt-icon name="trash" [size]="13" /></button>
              <button class="icon-btn" type="button" [attr.aria-label]="(open().has(i) ? 'Hide' : 'Edit') + ' step ' + (i + 1)" [attr.aria-expanded]="open().has(i)" title="Edit" (click)="toggle(i)"><imt-icon [name]="open().has(i) ? 'chevronDown' : 'chevronRight'" [size]="13" /></button>
            </span>
          </div>
          @if (open().has(i)) {
            <div class="card-body">
              @if (fieldsOf(s).length) {
                <div class="fields">
                  @for (f of fieldsOf(s); track f.key) {
                    <div class="lbl">
                      <span>{{ f.label }}</span>
                      @switch (f.control) {
                        @case ('list') {
                          <imt-string-list [label]="f.label" datalist="imt-profile-fields" [values]="listValue(s, f.key)" (changed)="setStep(i, f.key, $event)" />
                        }
                        @case ('number') {
                          <input class="input mono" type="number" min="0" [attr.aria-label]="f.label" [value]="textValue(s, f.key)" (input)="setNumber(i, f.key, $any($event.target).value)" />
                        }
                        @case ('level') {
                          <select class="select" [attr.aria-label]="f.label" [value]="s.level ?? 'Warning'" (change)="setStep(i, 'level', $any($event.target).value)">
                            @for (l of levels; track l) { <option [value]="l" [selected]="l === (s.level ?? 'Warning')">{{ l }}</option> }
                          </select>
                        }
                        @case ('plugin') {
                          <select class="select" [attr.aria-label]="f.label" [value]="s.id ?? ''" (change)="setStep(i, 'id', $any($event.target).value)">
                            <option value="" [selected]="!s.id">(choose a plugin)</option>
                            @for (p of pluginChoices(s); track p.id) { <option [value]="p.id" [selected]="p.id === s.id">{{ p.name }} ({{ p.id }})</option> }
                          </select>
                        }
                        @case ('json') {
                          <textarea class="input mono" rows="3" [attr.aria-label]="f.label" [value]="jsonValue(s, f.key)" (change)="setJson(i, f.key, $any($event.target).value)"></textarea>
                          @if (jsonError() === i) { <span class="bad" role="alert">That is not valid JSON; the table was left unchanged.</span> }
                        }
                        @case ('field') {
                          <input class="input mono" list="imt-profile-fields" [attr.aria-label]="f.label" [value]="textValue(s, f.key)" [placeholder]="f.placeholder ?? ''" (input)="setStep(i, f.key, $any($event.target).value)" />
                        }
                        @default {
                          <input class="input" [class.mono]="f.key === 'regex'" [attr.aria-label]="f.label" [value]="textValue(s, f.key)" [placeholder]="f.placeholder ?? ''" (input)="setStep(i, f.key, $any($event.target).value)" />
                        }
                      }
                    </div>
                  }
                </div>
              }
              <div class="lbl">
                <span>Only for events where <imt-help label="Conditions" text="All conditions must hold (AND). With none, the step applies to every event." /></span>
                <imt-conditions-editor [conditions]="s.when ?? []" emptyText="Applies to every event." (changed)="setStep(i, 'when', $event)" />
              </div>
            </div>
          }
        </div>
      } @empty {
        <div class="arrow" aria-hidden="true">↓</div>
        <div class="empty">No steps yet. Pick an activity above, for example "Drop" to remove noisy events.</div>
      }
      <div class="arrow" aria-hidden="true">↓</div>
      <div class="end-node">Indexed events</div>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; }
    h2 { margin: 0 0 4px; font-size: 16px; font-weight: 600; display: flex; align-items: center; gap: 8px; }
    .help { margin: 0; color: var(--muted); line-height: 1.5; max-width: 760px; }
    .small { font-size: 11px; }
    .palette { display: flex; flex-wrap: wrap; gap: 14px; padding: 10px; border: 1px solid var(--border-2); border-radius: 8px; background: var(--bg-2); }
    .pal-group { display: flex; flex-direction: column; gap: 6px; }
    .pal-items { display: flex; flex-wrap: wrap; gap: 4px; }
    .flow { display: flex; flex-direction: column; align-items: stretch; max-width: 900px; }
    .end-node { align-self: center; padding: 3px 14px; border: 1px dashed var(--border-3); border-radius: 12px; color: var(--muted); font-size: 12px; }
    .arrow { align-self: center; height: 22px; line-height: 22px; color: var(--faint); }
    .card { border: 1px solid var(--border-2); border-radius: 8px; background: var(--bg-2); }
    .card.off { opacity: 0.6; border-style: dashed; }
    .card-head { display: flex; align-items: center; gap: 8px; padding: 6px 8px; min-height: 38px; }
    .num { width: 20px; height: 20px; border-radius: 50%; background: var(--bg-active); color: var(--text-2); font-size: 11px; display: inline-flex; align-items: center; justify-content: center; flex: none; }
    .summary { flex: 1; min-width: 0; font-weight: 500; overflow-wrap: anywhere; }
    .stat { flex: none; font-size: 11px; padding: 2px 8px; border-radius: 10px; background: var(--bg-active); font-family: var(--font-mono); white-space: nowrap; }
    .stat.ok { color: var(--ok); }
    .stat.dim { color: var(--muted); }
    .actions { display: flex; align-items: center; gap: 2px; flex: none; }
    .off-toggle { display: inline-flex; align-items: center; gap: 4px; font-size: 11px; color: var(--muted); padding: 0 4px; }
    .icon-btn:disabled { opacity: 0.3; cursor: default; }
    .handle { cursor: grab; }
    .card-body { padding: 8px 12px 12px 40px; border-top: 1px solid var(--border); display: flex; flex-direction: column; gap: 10px; }
    .fields { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; }
    .lbl { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--muted); }
    .empty { align-self: center; color: var(--muted); padding: 8px; }
    .bad { color: var(--danger-text); }
    .cdk-drag-preview { box-shadow: var(--shadow); }
    .cdk-drag-placeholder { opacity: 0.3; }
  `,
})
export class PipelineEditor {
  private readonly draft = inject(ProfileDraft);
  private readonly store = inject(WorkspaceStore);

  protected readonly help = HELP;
  protected readonly levels = LEVELS;
  protected readonly open = signal<ReadonlySet<number>>(new Set());
  protected readonly jsonError = signal<number | null>(null);
  protected readonly steps = computed(() => this.draft.model().pipeline ?? []);
  protected readonly plugins = computed(() => transformPlugins(this.store.summary()?.plugins ?? []));
  protected readonly groups = STEP_GROUPS.map((name) => ({ name, kinds: STEP_CATALOG.filter((k) => k.group === name) }));

  protected icon = (kind: string) => kindInfo(kind)?.icon ?? 'settings';
  protected summary = summarizeStep;
  protected fieldsOf(s: PipelineStep): StepField[] {
    return STEP_FIELDS[s.step] ?? [];
  }
  protected status(s: PipelineStep, i: number) {
    return stepStatus(s, i, this.store.summary()?.pipeline ?? [], this.draft.dirty());
  }
  protected listValue(s: PipelineStep, key: string): string[] {
    const v = s[key];
    return Array.isArray(v) ? (v as string[]) : [];
  }
  protected textValue(s: PipelineStep, key: string): string {
    const v = s[key];
    return v === undefined || v === null ? '' : String(v);
  }
  protected jsonValue(s: PipelineStep, key: string): string {
    return s[key] === undefined ? '' : JSON.stringify(s[key], null, 2);
  }
  /** The installed transform plugins, plus the current id when its plugin is not loaded (so it stays visible). */
  protected pluginChoices(s: PipelineStep) {
    const known = this.plugins();
    return s.id && !known.some((p) => p.id === s.id) ? [...known, { id: s.id, name: 'not loaded', version: '', contributes: [] }] : known;
  }

  protected add(kind: PipelineStepKind): void {
    const index = this.steps().length;
    const step = defaultStep(kind, this.plugins()[0]?.id ?? '');
    this.draft.update((m) => (m.pipeline = [...(m.pipeline ?? []), step]));
    this.open.update((o) => new Set(o).add(index));
  }

  protected toggle(i: number): void {
    this.open.update((o) => {
      const next = new Set(o);
      if (!next.delete(i)) next.add(i);
      return next;
    });
  }

  protected setStep(i: number, key: string, value: unknown): void {
    this.draft.update((m) => {
      const step = m.pipeline?.[i];
      if (!step) return;
      if (value === undefined || value === '' || (Array.isArray(value) && value.length === 0)) delete step[key];
      else step[key] = value;
    });
  }

  protected setNumber(i: number, key: string, raw: string): void {
    const n = Number(raw);
    this.setStep(i, key, raw === '' || !Number.isFinite(n) ? undefined : n);
  }

  protected setJson(i: number, key: string, raw: string): void {
    if (!raw.trim()) {
      this.jsonError.set(null);
      this.setStep(i, key, undefined);
      return;
    }
    try {
      this.setStep(i, key, JSON.parse(raw));
      this.jsonError.set(null);
    } catch {
      this.jsonError.set(i);
    }
  }

  protected move(i: number, delta: number): void {
    this.reorder(i, i + delta);
  }

  protected drop(e: CdkDragDrop<unknown>): void {
    if (e.previousIndex !== e.currentIndex) this.reorder(e.previousIndex, e.currentIndex);
  }

  private reorder(from: number, to: number): void {
    this.draft.update((m) => (m.pipeline = moveItem(m.pipeline ?? [], from, to)));
    this.open.update((o) => new Set([...o].map((x) => remap(x, from, to))));
  }

  protected duplicate(i: number): void {
    this.draft.update((m) => (m.pipeline = duplicateStep(m.pipeline ?? [], i)));
    this.open.update((o) => new Set([...o].map((x) => (x > i ? x + 1 : x))));
  }

  protected remove(i: number): void {
    this.draft.update((m) => {
      const next = (m.pipeline ?? []).filter((_, j) => j !== i);
      if (next.length) m.pipeline = next;
      else delete m.pipeline;
    });
    this.open.update((o) => new Set([...o].filter((x) => x !== i).map((x) => (x > i ? x - 1 : x))));
  }
}

/** Where an open-card index ends up after moving an item `from` -> `to`. */
export function remap(index: number, from: number, to: number): number {
  if (index === from) return to;
  if (from < to && index > from && index <= to) return index - 1;
  if (from > to && index >= to && index < from) return index + 1;
  return index;
}
