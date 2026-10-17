import { CdkDrag, CdkDragDrop, CdkDragHandle, CdkDropList } from '@angular/cdk/drag-drop';
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

/** The "Markers" view: every marker rule in priority order with its look, meaning and live match count. */
@Component({
  selector: 'imt-markers-manager',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [MarkerGlyph, Icon, CdkDropList, CdkDrag, CdkDragHandle],
  template: `
    <div class="head">
      <span class="section-title">Markers</span>
      <span class="faint">First matching rule is an event's primary marker. Drag or use the arrows to reorder.</span>
      <span class="spacer"></span>
      <button type="button" class="btn primary" (click)="add()"><imt-icon name="plus" [size]="13" /> New marker</button>
    </div>
    @if (error(); as e) {
      <div class="err" role="alert">{{ e }} <button type="button" class="btn" (click)="reload()">Retry</button></div>
    }
    <div class="scroll">
      <table class="rules" aria-label="Marker rules">
        <thead>
          <tr>
            <th scope="col"><span class="sr">Reorder</span></th>
            <th scope="col"><span class="sr">Look</span></th>
            <th scope="col">Label</th>
            <th scope="col">Description</th>
            <th scope="col">Matches when</th>
            <th scope="col" class="num">Events</th>
            <th scope="col">On</th>
            <th scope="col"><span class="sr">Actions</span></th>
          </tr>
        </thead>
        <tbody cdkDropList (cdkDropListDropped)="dropped($event)">
          @for (r of rows(); track r.rule.kind; let i = $index) {
            <tr cdkDrag [class.off]="!r.enabled" [attr.data-kind]="r.rule.kind">
              <td><span cdkDragHandle class="grip" aria-hidden="true"><imt-icon name="drag" [size]="14" /></span></td>
              <td><imt-marker-glyph [glyph]="glyph(r.rule)" [size]="20" /></td>
              <th scope="row" class="lbl">{{ r.rule.label || r.rule.kind }}<span class="faint mono id">{{ r.rule.kind }}</span></th>
              <td class="desc">{{ r.rule.description }}</td>
              <td class="when">{{ when(r) }}</td>
              <td class="num mono">{{ r.enabled ? (counts()[r.rule.kind] === undefined ? '…' : fmt(counts()[r.rule.kind]!)) : '–' }}</td>
              <td><input type="checkbox" [checked]="r.enabled" [attr.aria-label]="(r.enabled ? 'Disable ' : 'Enable ') + (r.rule.label || r.rule.kind)" (change)="toggle(i)" /></td>
              <td class="actions">
                <button type="button" class="icon-btn" [disabled]="i === 0" [attr.aria-label]="'Move up ' + (r.rule.label || r.rule.kind)" (click)="move(i, i - 1)">↑</button>
                <button type="button" class="icon-btn" [disabled]="i === rows().length - 1" [attr.aria-label]="'Move down ' + (r.rule.label || r.rule.kind)" (click)="move(i, i + 1)">↓</button>
                <button type="button" class="btn" (click)="edit(r)" [attr.aria-label]="'Edit ' + (r.rule.label || r.rule.kind)">Edit</button>
                <button type="button" class="icon-btn" [attr.aria-label]="'Duplicate ' + (r.rule.label || r.rule.kind)" title="Duplicate" (click)="duplicate(i)"><imt-icon name="copy" [size]="13" /></button>
                @if (confirming() === r.rule.kind) {
                  <button type="button" class="btn danger" (click)="remove(i)">Confirm delete</button>
                  <button type="button" class="btn ghost" (click)="confirming.set(null)">Keep</button>
                } @else {
                  <button type="button" class="icon-btn" [attr.aria-label]="'Delete ' + (r.rule.label || r.rule.kind)" title="Delete" (click)="confirming.set(r.rule.kind)"><imt-icon name="trash" [size]="13" /></button>
                }
              </td>
            </tr>
          } @empty {
            <tr><td colspan="8" class="faint empty">{{ loaded() ? 'No marker rules. Add one to highlight lines on the timeline.' : 'Loading…' }}</td></tr>
          }
        </tbody>
      </table>

      @if (pluginKinds().length) {
        <h3 class="sub">Plugin markers <span class="faint">— contributed by plugins; their rules are read-only. Switch them off or change their look.</span></h3>
        <table class="rules plugin-kinds" aria-label="Plugin marker kinds">
          <thead>
            <tr>
              <th scope="col"><span class="sr">Look</span></th>
              <th scope="col">Label</th>
              <th scope="col">Plugin</th>
              <th scope="col">Description</th>
              <th scope="col">Matches when</th>
              <th scope="col" class="num">Events</th>
              <th scope="col">On</th>
              <th scope="col"><span class="sr">Actions</span></th>
            </tr>
          </thead>
          <tbody>
            @for (p of pluginKinds(); track p.kind) {
              <tr [class.off]="!p.enabled" [attr.data-kind]="p.kind">
                <td>
                  @if (p.info; as k) { <imt-marker-glyph [glyph]="{ kind: k.kind, color: k.color, icon: k.icon, badge: k.badge, shape: k.shape }" [size]="20" /> }
                </td>
                <th scope="row" class="lbl">{{ p.label }}<span class="faint mono id">{{ p.kind }}</span></th>
                <td><span class="badge owner mono" [title]="'Owned by the plugin ' + p.owner">{{ p.owner }}</span>@if (p.info?.role; as role) { <span class="badge">{{ role }}</span> }</td>
                <td class="desc">{{ p.info?.description }}</td>
                <td class="when">{{ p.when.length ? describe(p.when) : (p.enabled ? 'marked by the plugin' : '–') }} <span class="badge ro" title="Rules of plugin markers cannot be edited">read-only</span></td>
                <td class="num mono">{{ p.enabled && p.when.length ? (counts()[p.kind] === undefined ? '…' : fmt(counts()[p.kind]!)) : '–' }}</td>
                <td><input type="checkbox" [checked]="p.enabled" [attr.aria-label]="(p.enabled ? 'Disable ' : 'Enable ') + p.label" (change)="togglePlugin(p)" /></td>
                <td class="actions">
                  <button type="button" class="btn" [disabled]="!p.enabled" [attr.aria-label]="'Restyle ' + p.label" (click)="restyle(p)">Restyle</button>
                  @if (p.restyled.length) {
                    <button type="button" class="btn ghost" [attr.aria-label]="'Reset style of ' + p.label" (click)="resetPluginStyle(p)">Reset style</button>
                  }
                </td>
              </tr>
            }
          </tbody>
        </table>
      }
    </div>
  `,
  styles: `
    :host { flex: 1; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .head { display: flex; align-items: center; gap: 12px; padding: 8px 14px; font-size: 12px; }
    .spacer { flex: 1; }
    .scroll { flex: 1; overflow: auto; padding: 0 14px 14px; }
    .rules { width: 100%; border-collapse: collapse; font-size: 12.5px; }
    .rules th, .rules td { text-align: left; padding: 6px 8px; border-bottom: 1px solid var(--border); vertical-align: middle; }
    .rules thead th { color: var(--muted); font-weight: 500; position: sticky; top: 0; background: var(--bg-1); }
    .rules th.lbl { font-weight: 600; color: var(--text); }
    .id { display: block; font-weight: 400; font-size: 11px; }
    .desc, .when { color: var(--text-2); max-width: 280px; }
    .num { text-align: right; }
    .off { opacity: 0.6; }
    .grip { cursor: grab; color: var(--faint); display: inline-flex; }
    .actions { white-space: nowrap; }
    .actions > * { margin-right: 2px; }
    .empty { padding: 20px; text-align: center; }
    .err { padding: 8px 14px; color: var(--danger-text); }
    .sub { font-size: 12.5px; font-weight: 600; margin: 18px 0 6px; }
    .sub .faint { font-weight: 400; }
    .badge.owner { margin-right: 4px; }
    .badge.ro { font-size: 10px; margin-left: 4px; }
    .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
    .cdk-drag-preview { background: var(--bg-2); box-shadow: 0 4px 16px rgba(0,0,0,.35); }
  `,
})
export class MarkersManager {
  private readonly profile = inject(MarkerProfile);
  private readonly counter = inject(MatchCounter);
  private readonly dialogs = inject(DialogService);
  private readonly store = inject(WorkspaceStore);

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
      if (mine === this.seq) this.error.set(`Could not load the markers: ${e instanceof Error ? e.message : String(e)}`);
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
    this.error.set(`${res.failure.message}${res.failure.line ? ` (profile line ${res.failure.line})` : ''}`);
  }

  protected describe(when: PluginKindRow['when']): string {
    return describeWhen(when);
  }

  /** Writes a `[[marker_override]]` change through the same validate-and-save flow as the rules. */
  private async commitOverride(mutate: (model: ProfileModel) => void): Promise<void> {
    const res = await this.profile.commit((model) => mutate(model));
    if (res.ok) return void this.error.set(null);
    await this.reload();
    this.error.set(`${res.failure.message}${res.failure.line ? ` (profile line ${res.failure.line})` : ''}`);
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
      title: `Restyle "${p.label}"`,
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
    const kind = this.rows()[i].rule.kind;
    return this.commit((rows) => rows.map((r) => (r.rule.kind === kind ? { ...r, enabled: !r.enabled } : r)));
  }

  protected duplicate(i: number): Promise<void> {
    const kind = this.rows()[i].rule.kind;
    return this.commit((rows) => {
      const at = rows.findIndex((r) => r.rule.kind === kind);
      const src = rows[at];
      const copy: MarkerRuleX = { ...structuredClone(src.rule), kind: uniqueKind(rows, `${src.rule.kind}-copy`), label: `${src.rule.label ?? src.rule.kind} (copy)` };
      delete copy.badge;
      return [...rows.slice(0, at + 1), { rule: copy, enabled: src.enabled }, ...rows.slice(at + 1)];
    });
  }

  protected async remove(i: number): Promise<void> {
    const kind = this.rows()[i].rule.kind;
    this.confirming.set(null);
    await this.commit((rows) => rows.filter((r) => r.rule.kind !== kind));
  }

  protected edit(r: RuleRow): void {
    this.dialogs.open(MarkDialog, {
      title: `Edit marker "${r.rule.label || r.rule.kind}"`,
      inputs: { rule: r.rule, submitLabel: 'Save marker', hint: 'Change how this marker looks, what it means and which events it marks.' },
      width: 760,
    });
  }

  protected add(): void {
    this.dialogs.open(MarkDialog, {
      title: 'New marker',
      inputs: { initial: [{ field: '@message', op: 'matches', value: '' }], suggestedLabel: 'New marker', submitLabel: 'Add marker', hint: 'Describe the lines to highlight with a regex or other conditions.' },
      width: 760,
    });
  }
}
