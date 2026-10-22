import { CdkDrag, CdkDragDrop, CdkDragHandle, CdkDropList } from '@angular/cdk/drag-drop';
import { AiButton } from '../ai/ai-button';
import { AiLauncher } from '../ai/ai-launcher';
import { ChangeDetectionStrategy, Component, computed, effect, inject, signal, untracked } from '@angular/core';
import { MatchCounter } from '../context/match-count';
import { MarkDialog } from '../context/mark-dialog';
import { formatCount } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { Icon } from '../shared/icon';
import { DialogService } from '../ui/dialog.service';
import { MarkerGlyph } from './marker-glyph';
import { describeWhen, moveRow, pruneIcons, uniqueKind, writeRows } from './marker-rules';
import { MarkerProfile } from './marker-profile';
import { MarkerRuleX, RuleRow } from './markers.types';
import { MarkerOverrideDialog } from './marker-override-dialog';
import { OverrideStyle, PluginKindRow, pluginRows, readOverrides, resetStyle, setOverride } from './marker-overrides';
import { ProfileModel } from '../core/profile.model';
import { RunsEditor } from './runs-editor';

/** The engine's failure message, with the profile line it points at when it has one. */
function failureText(f: { message: string; line: number | null }): string {
  return f.line ? $localize`:@@markers.manager.failureLine:${f.message}:message: (profile line ${f.line}:line:)` : f.message;
}

/** The "Markers" view: every marker rule in priority order with its look, meaning and live match count. */
@Component({
  selector: 'imt-markers-manager',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [MarkerGlyph, Icon, CdkDropList, CdkDrag, CdkDragHandle, AiButton, RunsEditor],
  template: `
    <div class="head">
      <span class="section-title" i18n="@@markers.manager.title">Markers</span>
      <span class="faint" i18n="@@markers.manager.intro">First matching rule is an event's primary marker. Drag or use the arrows to reorder.</span>
      <span class="spacer"></span>
      <imt-ai-button label="Propose markers" i18n-label="@@markers.manager.propose" hint="The local model proposes markers from the selected event's file; the engine checks them" i18n-hint="@@markers.manager.proposeHint" testId="ai-propose-markers" (pressed)="proposeWithAi()" />
      <button type="button" class="btn primary" (click)="add()" i18n="@@markers.manager.new"><imt-icon name="plus" [size]="13" /> New marker</button>
    </div>
    @if (error(); as e) {
      <div class="err" role="alert">{{ e }} <button type="button" class="btn" (click)="reload()" i18n="@@markers.manager.retry">Retry</button></div>
    }
    <div class="scroll">
      <table class="rules" aria-label="Marker rules" i18n-aria-label="@@markers.manager.rules">
        <thead>
          <tr>
            <th scope="col"><span class="sr" i18n="@@markers.manager.col.reorder">Reorder</span></th>
            <th scope="col"><span class="sr" i18n="@@markers.manager.col.look">Look</span></th>
            <th scope="col" i18n="@@markers.manager.col.label">Label</th>
            <th scope="col" class="narrow-hide" i18n="@@markers.manager.col.description">Description</th>
            <th scope="col" class="narrow-hide" i18n="@@markers.manager.col.matchesWhen">Matches when</th>
            <th scope="col" class="num" i18n="@@markers.manager.col.events">Events</th>
            <th scope="col" i18n="@@markers.manager.col.on">On</th>
            <th scope="col"><span class="sr" i18n="@@markers.manager.col.actions">Actions</span></th>
          </tr>
        </thead>
        <tbody cdkDropList (cdkDropListDropped)="dropped($event)">
          @for (r of rows(); track r.rule.kind; let i = $index) {
            <tr cdkDrag [class.off]="!r.enabled" [attr.data-kind]="r.rule.kind">
              <td><span cdkDragHandle class="grip" aria-hidden="true"><imt-icon name="drag" [size]="14" /></span></td>
              <td><imt-marker-glyph [glyph]="glyph(r.rule)" [size]="20" /></td>
              <th scope="row" class="lbl" [title]="r.rule.description ?? ''">{{ r.rule.label || r.rule.kind }}<span class="faint mono id">{{ r.rule.kind }}</span></th>
              <td class="desc narrow-hide">{{ r.rule.description }}</td>
              <td class="when narrow-hide">{{ when(r) }}</td>
              <td class="num mono">{{ r.enabled ? (counts()[r.rule.kind] === undefined ? '…' : fmt(counts()[r.rule.kind]!)) : '–' }}</td>
              <td><input type="checkbox" [checked]="r.enabled" [attr.aria-label]="toggleLabel(r.enabled, r.rule.label || r.rule.kind)" (change)="toggle(i)" /></td>
              <td class="actions narrow-wrap">
                <button type="button" class="icon-btn" [disabled]="i === 0"  [attr.aria-label]="moveLabel('up', r.rule.label || r.rule.kind)" (click)="move(i, i - 1)">↑</button>
                <button type="button" class="icon-btn" [disabled]="i === rows().length - 1"  [attr.aria-label]="moveLabel('down', r.rule.label || r.rule.kind)" (click)="move(i, i + 1)">↓</button>
                <button type="button" class="btn" (click)="edit(r)" [attr.aria-label]="editLabel(r.rule.label || r.rule.kind)" i18n="@@markers.manager.edit">Edit</button>
                <button type="button" class="icon-btn" [attr.aria-label]="duplicateLabel(r.rule.label || r.rule.kind)" title="Duplicate" i18n-title="@@markers.manager.duplicate" (click)="duplicate(i)"><imt-icon name="copy" [size]="13" /></button>
                @if (confirming() === r.rule.kind) {
                  <button type="button" class="btn danger" (click)="remove(i)" i18n="@@markers.manager.confirmDelete">Confirm delete</button>
                  <button type="button" class="btn ghost" (click)="confirming.set(null)" i18n="@@markers.manager.keep">Keep</button>
                } @else {
                  <button type="button" class="icon-btn" [attr.aria-label]="deleteLabel(r.rule.label || r.rule.kind)" title="Delete" i18n-title="@@markers.manager.delete" (click)="confirming.set(r.rule.kind)"><imt-icon name="trash" [size]="13" /></button>
                }
              </td>
            </tr>
          } @empty {
            <tr><td colspan="8" class="faint empty">@if (loaded()) { <ng-container i18n="@@markers.manager.empty">No marker rules. Add one to highlight lines on the timeline.</ng-container> } @else { <ng-container i18n="@@markers.manager.loading">Loading…</ng-container> }</td></tr>
          }
        </tbody>
      </table>

      @if (pluginKinds().length) {
        <h3 class="sub" i18n="@@markers.manager.plugin.title">Plugin markers <span class="faint">— contributed by plugins; their rules are read-only. Switch them off or change their look.</span></h3>
        <table class="rules plugin-kinds" aria-label="Plugin marker kinds" i18n-aria-label="@@markers.manager.plugin.kinds">
          <thead>
            <tr>
              <th scope="col"><span class="sr" i18n="@@markers.manager.col.look">Look</span></th>
              <th scope="col" i18n="@@markers.manager.col.label">Label</th>
              <th scope="col" i18n="@@markers.manager.col.plugin">Plugin</th>
              <th scope="col" class="narrow-hide" i18n="@@markers.manager.col.description">Description</th>
              <th scope="col" class="narrow-hide" i18n="@@markers.manager.col.matchesWhen">Matches when</th>
              <th scope="col" class="num" i18n="@@markers.manager.col.events">Events</th>
              <th scope="col" i18n="@@markers.manager.col.on">On</th>
              <th scope="col"><span class="sr" i18n="@@markers.manager.col.actions">Actions</span></th>
            </tr>
          </thead>
          <tbody>
            @for (p of pluginKinds(); track p.kind) {
              <tr [class.off]="!p.enabled" [attr.data-kind]="p.kind">
                <td>
                  @if (p.info; as k) { <imt-marker-glyph [glyph]="{ kind: k.kind, color: k.color, icon: k.icon, badge: k.badge, shape: k.shape }" [size]="20" /> }
                </td>
                <th scope="row" class="lbl" [title]="p.info?.description ?? ''">{{ p.label }}<span class="faint mono id">{{ p.kind }}</span></th>
                <td><span class="badge owner mono" [title]="ownerTitle(p.owner)">{{ p.owner }}</span>@if (p.info?.role; as role) { <span class="badge">{{ role }}</span> }</td>
                <td class="desc narrow-hide">{{ p.info?.description }}</td>
                <td class="when narrow-hide">{{ pluginWhen(p) }} <span class="badge ro" title="Rules of plugin markers cannot be edited" i18n-title="@@markers.manager.readOnlyTitle" i18n="@@markers.manager.readOnly">read-only</span></td>
                <td class="num mono">{{ p.enabled && p.when.length ? (counts()[p.kind] === undefined ? '…' : fmt(counts()[p.kind]!)) : '–' }}</td>
                <td><input type="checkbox" [checked]="p.enabled" [attr.aria-label]="toggleLabel(p.enabled, p.label)" (change)="togglePlugin(p)" /></td>
                <td class="actions narrow-wrap">
                  <button type="button" class="btn" [disabled]="!p.enabled" [attr.aria-label]="restyleLabel(p.label)" (click)="restyle(p)" i18n="@@markers.manager.restyle">Restyle</button>
                  @if (p.restyled.length) {
                    <button type="button" class="btn ghost" [attr.aria-label]="resetLabel(p.label)" (click)="resetPluginStyle(p)" i18n="@@markers.manager.resetStyle">Reset style</button>
                  }
                </td>
              </tr>
            }
          </tbody>
        </table>
      }
      <imt-runs-editor />
    </div>
  `,
  styles: `
    :host { flex: 1; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .head { display: flex; align-items: center; gap: 12px; padding: 8px 14px; font-size: var(--fs-sm); }
    .spacer { flex: 1; }
    .scroll { flex: 1; overflow: auto; padding: 0 14px 14px; }
    .rules { width: 100%; border-collapse: collapse; font-size: var(--fs-md); }
    .rules th, .rules td { text-align: left; padding: 6px 8px; border-bottom: 1px solid var(--border); vertical-align: middle; }
    .rules thead th { color: var(--muted); font-weight: 500; position: sticky; top: 0; background: var(--bg-1); }
    .rules th.lbl { font-weight: 600; color: var(--text); }
    .id { display: block; font-weight: 400; font-size: var(--fs-xs); }
    .desc, .when { color: var(--text-2); max-width: 280px; }
    .num { text-align: right; }
    .off { opacity: 0.6; }
    .grip { cursor: grab; color: var(--faint); display: inline-flex; }
    .actions { white-space: nowrap; }
    .actions > * { margin-right: 2px; }
    .empty { padding: 20px; text-align: center; }
    .err { padding: 8px 14px; color: var(--danger-text); }
    .sub { font-size: var(--fs-md); font-weight: 600; margin: 18px 0 6px; }
    .sub .faint { font-weight: 400; }
    .badge.owner { margin-right: 4px; }
    .badge.ro { font-size: var(--fs-2xs); margin-left: 4px; }
    .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
    .cdk-drag-preview { background: var(--bg-2); box-shadow: var(--shadow-pop); }
  `,
})
export class MarkersManager {
  private readonly profile = inject(MarkerProfile);
  private readonly counter = inject(MatchCounter);
  private readonly dialogs = inject(DialogService);
  private readonly store = inject(WorkspaceStore);
  private readonly ai = inject(AiLauncher);

  protected readonly rows = signal<RuleRow[]>([]);
  protected readonly counts = signal<Record<string, number | undefined>>({});
  protected readonly error = signal<string | null>(null);
  protected readonly loaded = signal(false);
  protected readonly confirming = signal<string | null>(null);
  /** The profile the rows came from (for `[[marker_override]]`). */
  private readonly model = signal<ProfileModel>({});
  /** Plugin-owned kinds (the engine's summary) plus those the profile switched off. */
  protected readonly pluginKinds = computed<PluginKindRow[]>(() => pluginRows(this.store.summary()?.markerKinds ?? [], this.model()));
  protected readonly fmt = formatCount;
  private seq = 0;

  constructor() {
    // Reload whenever the workspace summary changes (a save re-imports and publishes a new summary).
    effect(() => {
      this.store.summary();
      untracked(() => void this.reload());
    });
  }

  protected glyph(rule: MarkerRuleX) {
    return { kind: rule.kind, color: rule.color ?? null, icon: rule.icon, badge: rule.badge, shape: rule.shape };
  }

  protected toggleLabel(enabled: boolean, name: string): string {
    return enabled ? $localize`:@@markers.manager.disableAria:Disable ${name}:name:` : $localize`:@@markers.manager.enableAria:Enable ${name}:name:`;
  }
  protected moveLabel(dir: 'up' | 'down', name: string): string {
    return dir === 'up' ? $localize`:@@markers.manager.moveUpAria:Move up ${name}:name:` : $localize`:@@markers.manager.moveDownAria:Move down ${name}:name:`;
  }
  protected editLabel(name: string): string {
    return $localize`:@@markers.manager.editAria:Edit ${name}:name:`;
  }
  protected duplicateLabel(name: string): string {
    return $localize`:@@markers.manager.duplicateAria:Duplicate ${name}:name:`;
  }
  protected deleteLabel(name: string): string {
    return $localize`:@@markers.manager.deleteAria:Delete ${name}:name:`;
  }
  protected restyleLabel(name: string): string {
    return $localize`:@@markers.manager.restyleAria:Restyle ${name}:name:`;
  }
  protected resetLabel(name: string): string {
    return $localize`:@@markers.manager.resetAria:Reset style of ${name}:name:`;
  }
  protected ownerTitle(owner: string): string {
    return $localize`:@@markers.manager.ownerTitle:Owned by the plugin ${owner}:owner:`;
  }
  protected pluginWhen(p: PluginKindRow): string {
    if (p.when.length) return this.describe(p.when);
    return p.enabled ? $localize`:@@markers.manager.markedByPlugin:marked by the plugin` : '–';
  }

  protected when(r: RuleRow): string {
    return describeWhen(r.rule.when);
  }

  protected async reload(): Promise<void> {
    const mine = ++this.seq;
    try {
      const { rows, model } = await this.profile.load();
      if (mine !== this.seq) return;
      this.rows.set(rows);
      this.model.set(model);
      this.error.set(null);
      this.loaded.set(true);
      void this.count(rows, mine);
    } catch (e) {
      if (mine === this.seq) this.error.set($localize`:@@markers.manager.loadFailed:Could not load the markers: ${e instanceof Error ? e.message : String(e)}:error:`);
    }
  }

  private async count(rows: RuleRow[], mine: number): Promise<void> {
    const plugin = untracked(() => this.pluginKinds()).filter((p) => p.enabled && p.when.length);
    const targets = [...rows.filter((r) => r.enabled).map((r) => ({ kind: r.rule.kind, when: r.rule.when ?? [] })), ...plugin];
    await Promise.all(
      targets.map(async (r) => {
        try {
          const n = await this.counter.count(r.when);
          if (mine === this.seq) this.counts.update((c) => ({ ...c, [r.kind]: n }));
        } catch {
          /* a failed count leaves the cell on "…" */
        }
      }),
    );
  }

  private async commit(mutate: (rows: RuleRow[]) => RuleRow[]): Promise<void> {
    const res = await this.profile.commit((model, rows) => {
      const next = mutate(rows);
      writeRows(model, next);
      pruneIcons(model, next);
    });
    if (res.ok) return void this.error.set(null);
    await this.reload(); // roll the list back to what is saved, then say why
    this.error.set(failureText(res.failure));
  }

  protected describe(when: PluginKindRow['when']): string {
    return describeWhen(when);
  }

  /** Writes a `[[marker_override]]` change through the same validate-and-save flow as the rules. */
  private async commitOverride(mutate: (model: ProfileModel) => void): Promise<void> {
    const res = await this.profile.commit((model) => mutate(model));
    if (res.ok) return void this.error.set(null);
    await this.reload();
    this.error.set(failureText(res.failure));
  }

  protected togglePlugin(p: PluginKindRow): Promise<void> {
    return this.commitOverride((model) => setOverride(model, p.kind, { enabled: !p.enabled }));
  }

  protected resetPluginStyle(p: PluginKindRow): Promise<void> {
    return this.commitOverride((model) => resetStyle(model, p.kind));
  }

  protected async restyle(p: PluginKindRow): Promise<void> {
    const current = readOverrides(this.model()).find((o) => o.kind === p.kind) ?? { kind: p.kind };
    const ref = this.dialogs.open<OverrideStyle>(MarkerOverrideDialog, {
      title: $localize`:@@markers.manager.restyleTitle:Restyle "${p.label}:label:"`,
      inputs: { row: p, current },
      width: 560,
    });
    const style = await ref.closed;
    if (style) await this.commitOverride((model) => setOverride(model, p.kind, style));
  }

  protected move(from: number, to: number): Promise<void> {
    const kind = this.rows()[from]?.rule.kind;
    this.rows.update((r) => moveRow(r, from, to));
    return this.commit((rows) => moveRow(rows, rows.findIndex((r) => r.rule.kind === kind), to));
  }

  protected dropped(e: CdkDragDrop<unknown>): Promise<void> {
    return this.move(e.previousIndex, e.currentIndex);
  }

  protected toggle(i: number): Promise<void> {
    const kind = this.rows()[i]?.rule.kind;
    if (kind === undefined) return Promise.resolve();
    return this.commit((rows) => rows.map((r) => (r.rule.kind === kind ? { ...r, enabled: !r.enabled } : r)));
  }

  protected duplicate(i: number): Promise<void> {
    const kind = this.rows()[i]?.rule.kind;
    if (kind === undefined) return Promise.resolve();
    return this.commit((rows) => {
      const at = rows.findIndex((r) => r.rule.kind === kind);
      const src = rows[at];
      if (!src) return rows;
      const copy: MarkerRuleX = { ...structuredClone(src.rule), kind: uniqueKind(rows, `${src.rule.kind}-copy`), label: $localize`:@@markers.manager.copyLabel:${src.rule.label ?? src.rule.kind}:name: (copy)` };
      delete copy.badge;
      return [...rows.slice(0, at + 1), { rule: copy, enabled: src.enabled }, ...rows.slice(at + 1)];
    });
  }

  protected async remove(i: number): Promise<void> {
    const kind = this.rows()[i]?.rule.kind;
    if (kind === undefined) return;
    this.confirming.set(null);
    await this.commit((rows) => rows.filter((r) => r.rule.kind !== kind));
  }

  protected edit(r: RuleRow): void {
    this.dialogs.open(MarkDialog, {
      title: $localize`:@@markers.manager.editTitle:Edit marker "${r.rule.label || r.rule.kind}:name:"`,
      inputs: { rule: r.rule, submitLabel: $localize`:@@markers.manager.editSubmit:Save marker`,
        hint: $localize`:@@markers.manager.editHint:Change how this marker looks, what it means and which events it marks.`,
      },
      width: 760,
    });
  }

  protected add(): void {
    this.dialogs.open(MarkDialog, {
      title: $localize`:@@markers.manager.newTitle:New marker`,
      inputs: {
        initial: [{ field: '@message', op: 'matches', value: '' }],
        suggestedLabel: $localize`:@@markers.manager.newLabel:New marker`,
        submitLabel: $localize`:@@markers.manager.newSubmit:Add marker`,
        hint: $localize`:@@markers.manager.newHint:Describe the lines to highlight with a regex or other conditions.`,
      },
      width: 760,
    });
  }

  /** "Propose markers" with the local AI model, on the selected event's file. */
  protected proposeWithAi(): void {
    void this.ai.propose('markers', { recordId: this.store.selected()?.id ?? null });
  }
}
