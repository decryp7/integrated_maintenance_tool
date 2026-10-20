import { CdkDrag, CdkDragDrop, CdkDragHandle, CdkDropList } from '@angular/cdk/drag-drop';
import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { LEVELS } from '../core/api.types';
import { PipelineStep, PipelineStepKind } from '../core/profile.model';
import { WorkspaceStore } from '../core/workspace.store';
import { Icon } from '../shared/icon';
import { HelpHint } from '../ui/help-hint';
import { ConditionsEditor } from './conditions-editor';
import { ProfileDraft } from './profile-draft';
import { STEP_FIELDS, StepField } from './pipeline-fields';
import {
  defaultStep,
  duplicateStep,
  kindInfo,
  moveItem,
  SCRIPT_HELP,
  STEP_CATALOG,
  STEP_GROUP_LABELS,
  STEP_GROUPS,
  stepProblem,
  stepStatus,
  summarizeStep,
  transformPlugins,
} from './pipeline-model';
import { StringList } from './string-list';

const HELP = $localize`:@@profile.pipelineEditor.help:The pipeline runs on every event after it was parsed and before it is indexed, top to bottom. Use it to drop noise, hide secrets, fix levels or add fields. Steps can be reordered, duplicated and disabled.`;

/** The ordered `[[pipeline]]` as an activity flow: a palette of step kinds and a vertical chain of cards. */
@Component({
  selector: 'imt-pipeline-editor',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, ConditionsEditor, StringList, HelpHint, CdkDropList, CdkDrag, CdkDragHandle],
  template: `
    <h2><ng-container i18n="@@profile.pipelineEditor.title">Pipeline</ng-container> <imt-help label="Pipeline" i18n-label="@@profile.pipelineEditor.helpLabel" [text]="help" /></h2>
    <p class="help" i18n="@@profile.pipelineEditor.intro">Each event flows through these steps in order after parsing. Add an activity, then fill in what it should do.</p>

    <div class="palette" role="group" aria-label="Add an activity" i18n-aria-label="@@profile.pipelineEditor.palette">
      @for (g of groups; track g.name) {
        <div class="pal-group">
          <span class="section-title">{{ g.label }}</span>
          <div class="pal-items">
            @for (k of g.kinds; track k.kind) {
              <button class="btn" type="button" [title]="k.blurb" [disabled]="k.kind === 'plugin' && !plugins().length" (click)="add(k.kind)">
                <imt-icon [name]="k.icon" [size]="13" /> {{ k.label }}
              </button>
            }
          </div>
          @if (g.name === 'Extend' && !plugins().length) {
            <span class="faint small" i18n="@@profile.pipelineEditor.noPlugins">No transform plugins are loaded.</span>
          }
        </div>
      }
    </div>

    <div class="flow" cdkDropList (cdkDropListDropped)="drop($event)" aria-label="Pipeline steps" i18n-aria-label="@@profile.pipelineEditor.steps">
      <div class="end-node" i18n="@@profile.pipelineEditor.parsed">Parsed events</div>
      @for (s of steps(); track $index; let i = $index) {
        <div class="arrow" aria-hidden="true">↓</div>
        <div class="card" [class.off]="s.disabled" cdkDrag cdkDragLockAxis="y" [attr.data-step]="s.step">
          <div class="card-head">
            <button class="icon-btn handle" type="button" cdkDragHandle [attr.aria-label]="stepLabel('drag', i)" title="Drag to reorder" i18n-title="@@profile.pipelineEditor.dragTitle"><imt-icon name="drag" [size]="14" [stroke]="3" /></button>
            <span class="num">{{ i + 1 }}</span>
            <imt-icon [name]="icon(s.step)" [size]="15" />
            <span class="summary">{{ summary(s) }}</span>
            <span class="stat" [class.ok]="status(s, i).state === 'ok'" [class.bad]="status(s, i).state === 'failed'" [class.dim]="status(s, i).state === 'stale' || status(s, i).state === 'disabled'" [attr.data-state]="status(s, i).state">{{ status(s, i).text }}</span>
            <span class="actions">
              <button class="icon-btn" type="button" [attr.aria-label]="stepLabel('up', i)" title="Move up" i18n-title="@@profile.pipelineEditor.moveUp" [disabled]="i === 0" (click)="move(i, -1)">↑</button>
              <button class="icon-btn" type="button" [attr.aria-label]="stepLabel('down', i)" title="Move down" i18n-title="@@profile.pipelineEditor.moveDown" [disabled]="i === steps().length - 1" (click)="move(i, 1)">↓</button>
              <button class="icon-btn" type="button" [attr.aria-label]="stepLabel('duplicate', i)" title="Duplicate" i18n-title="@@profile.pipelineEditor.duplicate" (click)="duplicate(i)"><imt-icon name="copy" [size]="13" /></button>
              <label class="off-toggle" [title]="offTitle(s.disabled)" i18n="@@profile.pipelineEditor.off">
                <input type="checkbox" [checked]="!!s.disabled" [attr.aria-label]="stepLabel('disable', i)" (change)="setStep(i, 'disabled', $any($event.target).checked || undefined)" /> off
              </label>
              <button class="icon-btn" type="button" [attr.aria-label]="stepLabel('remove', i)" title="Remove" i18n-title="@@profile.pipelineEditor.remove" (click)="remove(i)"><imt-icon name="trash" [size]="13" /></button>
              <button class="icon-btn" type="button" [attr.aria-label]="stepLabel(open().has(i) ? 'hide' : 'edit', i)" [attr.aria-expanded]="open().has(i)" title="Edit" i18n-title="@@profile.pipelineEditor.edit" (click)="toggle(i)"><imt-icon [name]="open().has(i) ? 'chevronDown' : 'chevronRight'" [size]="13" /></button>
            </span>
          </div>
          @if (problem(i); as p) {
            <div class="problem bad" role="alert" data-testid="step-problem">{{ p }}</div>
          }
          @if (open().has(i)) {
            <div class="card-body">
              @if (fieldsOf(s).length) {
                <div class="fields">
                  @for (f of fieldsOf(s); track f.key) {
                    <div class="lbl" [class.wide]="f.control === 'script'">
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
                            <option value="" [selected]="!s.id" i18n="@@profile.pipelineEditor.choosePlugin">(choose a plugin)</option>
                            @for (p of pluginChoices(s); track p.id) { <option [value]="p.id" [selected]="p.id === s.id">{{ p.name }} ({{ p.id }})</option> }
                          </select>
                        }
                        @case ('json') {
                          <textarea class="input mono" rows="3" [attr.aria-label]="f.label" [value]="jsonValue(s, f.key)" (change)="setJson(i, f.key, $any($event.target).value)"></textarea>
                          @if (jsonError() === i) { <span class="bad" role="alert" i18n="@@profile.pipelineEditor.badJson">That is not valid JSON; the table was left unchanged.</span> }
                        }
                        @case ('script') {
                          <textarea class="input mono script" rows="8" spellcheck="false" autocapitalize="off" [attr.aria-label]="f.label" [attr.aria-describedby]="'imt-script-help-' + i" [attr.aria-invalid]="!!problem(i)" [value]="textValue(s, f.key)" (input)="setStep(i, f.key, $any($event.target).value)"></textarea>
                          <span class="faint small" [id]="'imt-script-help-' + i">{{ scriptHelp }}</span>
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
                <span><ng-container i18n="@@profile.pipelineEditor.onlyFor">Only for events where</ng-container>&ngsp;<imt-help label="Conditions" i18n-label="@@profile.pipelineEditor.conditionsLabel" text="All conditions must hold (AND). With none, the step applies to every event." i18n-text="@@profile.pipelineEditor.conditionsHelp" /></span>
                <imt-conditions-editor [conditions]="s.when ?? []" emptyText="Applies to every event." i18n-emptyText="@@profile.conditions.everyEvent" (changed)="setStep(i, 'when', $event)" />
              </div>
            </div>
          }
        </div>
      } @empty {
        <div class="arrow" aria-hidden="true">↓</div>
        <div class="empty" i18n="@@profile.pipelineEditor.empty">No steps yet. Pick an activity above, for example "Drop" to remove noisy events.</div>
      }
      <div class="arrow" aria-hidden="true">↓</div>
      <div class="end-node" i18n="@@profile.pipelineEditor.indexed">Indexed events</div>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; }
    h2 { margin: 0 0 4px; font-size: var(--fs-lg); font-weight: 600; display: flex; align-items: center; gap: 8px; }
    .help { margin: 0; color: var(--muted); line-height: 1.5; max-width: 760px; }
    .small { font-size: var(--fs-xs); }
    .palette { display: flex; flex-wrap: wrap; gap: 14px; padding: 10px; border: 1px solid var(--border-2); border-radius: 8px; background: var(--bg-2); }
    .pal-group { display: flex; flex-direction: column; gap: 6px; }
    .pal-items { display: flex; flex-wrap: wrap; gap: 4px; }
    .flow { display: flex; flex-direction: column; align-items: stretch; max-width: 900px; }
    .end-node { align-self: center; padding: 3px 14px; border: 1px dashed var(--border-3); border-radius: 12px; color: var(--muted); font-size: var(--fs-sm); }
    .arrow { align-self: center; height: 22px; line-height: 22px; color: var(--faint); }
    .card { border: 1px solid var(--border-2); border-radius: 8px; background: var(--bg-2); }
    .card.off { opacity: 0.6; border-style: dashed; }
    .card-head { display: flex; align-items: center; gap: 8px; padding: 6px 8px; min-height: 38px; }
    .num { width: 20px; height: 20px; border-radius: 50%; background: var(--bg-active); color: var(--text-2); font-size: var(--fs-xs); display: inline-flex; align-items: center; justify-content: center; flex: none; }
    .summary { flex: 1; min-width: 0; font-weight: 500; overflow-wrap: anywhere; }
    .stat { flex: none; font-size: var(--fs-xs); padding: 2px 8px; border-radius: 10px; background: var(--bg-active); font-family: var(--font-mono); white-space: nowrap; }
    .stat.ok { color: var(--ok); }
    .stat.dim { color: var(--muted); }
    .actions { display: flex; align-items: center; gap: 2px; flex: none; }
    .off-toggle { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-xs); color: var(--muted); padding: 0 4px; }
    .icon-btn:disabled { opacity: 0.3; cursor: default; }
    .handle { cursor: grab; }
    .card-body { padding: 8px 12px 12px 40px; border-top: 1px solid var(--border); display: flex; flex-direction: column; gap: 10px; }
    .fields { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; }
    .lbl { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-sm); color: var(--muted); }
    .empty { align-self: center; color: var(--muted); padding: 8px; }
    .bad { color: var(--danger-text); }
    .lbl.wide { grid-column: 1 / -1; }
    textarea.script { min-height: 120px; resize: vertical; white-space: pre; tab-size: 4; }
    .problem { padding: 0 12px 8px 40px; font-family: var(--font-mono); font-size: var(--fs-sm); white-space: pre-wrap; overflow-wrap: anywhere; }
    .cdk-drag-preview { box-shadow: var(--shadow); }
    .cdk-drag-placeholder { opacity: 0.3; }
  `,
})
export class PipelineEditor {
  private readonly draft = inject(ProfileDraft);
  private readonly store = inject(WorkspaceStore);

  protected readonly help = HELP;
  protected readonly scriptHelp = SCRIPT_HELP;
  protected readonly levels = LEVELS;
  protected readonly open = signal<ReadonlySet<number>>(new Set());
  protected readonly jsonError = signal<number | null>(null);
  protected readonly steps = computed(() => this.draft.model().pipeline ?? []);
  protected readonly plugins = computed(() => transformPlugins(this.store.summary()?.plugins ?? []));
  protected readonly groups = STEP_GROUPS.map((name) => ({ name, label: STEP_GROUP_LABELS[name], kinds: STEP_CATALOG.filter((k) => k.group === name) }));

  protected stepLabel(action: 'drag' | 'up' | 'down' | 'duplicate' | 'disable' | 'remove' | 'hide' | 'edit', i: number): string {
    const n = i + 1;
    switch (action) {
      case 'drag':
        return $localize`:@@profile.pipelineEditor.dragAria:Drag step ${n}:number: to reorder`;
      case 'up':
        return $localize`:@@profile.pipelineEditor.upAria:Move step ${n}:number: up`;
      case 'down':
        return $localize`:@@profile.pipelineEditor.downAria:Move step ${n}:number: down`;
      case 'duplicate':
        return $localize`:@@profile.pipelineEditor.duplicateAria:Duplicate step ${n}:number:`;
      case 'disable':
        return $localize`:@@profile.pipelineEditor.disableAria:Disable step ${n}:number:`;
      case 'remove':
        return $localize`:@@profile.pipelineEditor.removeAria:Remove step ${n}:number:`;
      case 'hide':
        return $localize`:@@profile.pipelineEditor.hideAria:Hide step ${n}:number:`;
      case 'edit':
        return $localize`:@@profile.pipelineEditor.editAria:Edit step ${n}:number:`;
    }
  }
  protected offTitle(disabled: boolean | undefined): string {
    return disabled ? $localize`:@@profile.pipelineEditor.enableTitle:Enable this step` : $localize`:@@profile.pipelineEditor.disableTitle:Disable this step (kept in the profile)`;
  }
  protected icon = (kind: string) => kindInfo(kind)?.icon ?? 'settings';
  protected summary = summarizeStep;
  protected fieldsOf(s: PipelineStep): StepField[] {
    return STEP_FIELDS[s.step] ?? [];
  }
  protected status(s: PipelineStep, i: number) {
    return stepStatus(s, i, this.store.summary()?.pipeline ?? [], this.draft.dirty());
  }
  /** The engine's validation error about step `i`, shown on its card. */
  protected problem(i: number): string | null {
    return stepProblem(this.draft.problem()?.message, i);
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
    return s.id && !known.some((p) => p.id === s.id) ? [...known, { id: s.id, name: $localize`:@@profile.pipelineEditor.notLoaded:not loaded`, version: '', contributes: [] }] : known;
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
