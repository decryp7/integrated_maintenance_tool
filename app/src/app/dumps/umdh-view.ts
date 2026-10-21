import { CdkTree, CdkTreeModule } from '@angular/cdk/tree';
import { ChangeDetectionStrategy, Component, computed, effect, inject, signal, untracked, viewChild } from '@angular/core';
import { DumpDocSummary, DumpFinding, UmdhDiffPage, UmdhGroup, UmdhStackDto, UmdhTreeNode } from '../core/api.types';
import { AnalysisState } from '../analysis/analysis-state';
import { AsyncResource, errorText } from '../analysis/async-resource';
import { DumpsBackend } from './dumps-backend';
import { DumpFindings, DumpsExplainer, DUMPS_STYLES, formatBytes, formatCount, formatCountDelta, formatDelta, NoDump } from './dumps-shared';
import { DumpsStore } from './dumps.store';

/** Rows of the growth table asked for at once. */
const PAGE = 200;
/** Children of a tree node asked for at once. */
const TREE_LIMIT = 100;

/** A node of the stack tree as the CDK tree shows it (flat, with its level). */
export interface TreeItem {
  node: UmdhTreeNode;
  /** Frame keys from the outermost caller to this node. */
  path: string[];
  level: number;
}

/** The flat tree with `children` inserted after `parent` (replacing what was there). Pure. */
export function insertChildren(items: readonly TreeItem[], parent: TreeItem, children: readonly UmdhTreeNode[]): TreeItem[] {
  const at = items.findIndex((i) => i.path.join('\n') === parent.path.join('\n'));
  if (at < 0) return [...items];
  let end = at + 1;
  while (end < items.length && items[end]!.level > parent.level) end++;
  const kids = children.map((node) => ({ node, path: [...parent.path, node.key], level: parent.level + 1 }));
  return [...items.slice(0, at + 1), ...kids, ...items.slice(end)];
}

/**
 * UMDH heap growth, problem first: the engine's leak findings over the open snapshots, then the growth between two
 * logs grouped by stack, module or function, a stack's frames, and the top-down tree of growing stacks.
 */
@Component({
  selector: 'imt-umdh-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, CdkTreeModule, DumpFindings, NoDump],
  template: `
    @if (store.umdh().length) {
      <div class="bar">
        <label class="inline"><span i18n="@@umdh.target">Log</span>
          <select data-testid="umdh-target" [value]="store.umdhTarget()" (change)="setTarget($event)">
            @for (d of store.umdh(); track d.id) { <option [value]="d.id">{{ d.name }}</option> }
          </select>
        </label>
        @if (target()?.kind === 'umdhSnapshot') {
          <label class="inline"><span i18n="@@umdh.base">compared with</span>
            <select data-testid="umdh-base" [value]="store.umdhBase() ?? ''" (change)="setBase($event)">
              <option value="" i18n="@@umdh.base.none">nothing (all allocations)</option>
              @for (d of snapshots(); track d.id) {
                @if (d.id !== store.umdhTarget()) { <option [value]="d.id">{{ d.name }}</option> }
              }
            </select>
          </label>
        }
        <span class="spacer"></span>
        <button class="btn ghost" type="button" (click)="closeTarget()" i18n="@@umdh.close">Close</button>
      </div>
      @if (store.error(); as e) { <div class="notice err" role="alert">{{ e }}</div> }
      <div class="tabs" role="tablist" aria-label="UMDH views" i18n-aria-label="@@umdh.tabs">
        @for (t of tabs; track t.id) {
          <button type="button" role="tab" [attr.aria-selected]="tab() === t.id" [attr.data-testid]="'umdh-tab-' + t.id" (click)="tab.set(t.id)">{{ t.label }}</button>
        }
      </div>
      <div class="panel" role="tabpanel">
        @switch (tab()) {
          @case ('findings') {
            <imt-dump-findings [res]="findings" emptyText="No leak suspects: nothing grows past the thresholds." i18n-emptyText="@@umdh.findings.empty" (evidence)="showStack($event)" (explain)="explain()" />
          }
          @case ('growth') {
            <div class="bar">
              <div role="radiogroup" class="group" aria-label="Group by" i18n-aria-label="@@umdh.groupBy">
                @for (g of groups; track g.id) {
                  <label class="inline"><input type="radio" name="umdh-group" [checked]="group() === g.id" (change)="group.set(g.id)" [attr.data-testid]="'umdh-group-' + g.id" /> {{ g.label }}</label>
                }
              </div>
              <input type="search" [value]="filter()" (input)="filter.set(value($event))" placeholder="Filter" i18n-placeholder="@@umdh.filter" aria-label="Filter growth" i18n-aria-label="@@umdh.filterLabel" />
              @if (diff.data(); as d) {
                <span class="faint" i18n="@@umdh.total">{{ delta(d.deltaBytes) }} in all, {{ bytes(d.bytes) }} alive</span>
              }
            </div>
            <div class="split">
              <div class="main" tabindex="0" role="region" aria-label="Growth table" i18n-aria-label="@@umdh.growth.region">
                <imt-analysis-state [loading]="diff.loading()" [error]="diff.error()" [retryable]="diff.retryable()" [hasData]="!!diff.data()" [empty]="diff.data()?.total === 0" emptyText="No allocations." i18n-emptyText="@@umdh.growth.empty" (retry)="diff.retry()">
                  <table class="grid" data-testid="umdh-growth" aria-label="Growth" i18n-aria-label="@@umdh.growth.label">
                    <thead><tr><th>{{ groupLabel() }}</th><th class="num" i18n="@@umdh.growth.delta">Growth</th><th class="num" i18n="@@umdh.growth.deltaCount">Allocations</th><th class="num" i18n="@@umdh.growth.bytes">Alive</th><th class="num" i18n="@@umdh.growth.stacks">Stacks</th></tr></thead>
                    <tbody>
                      @for (r of diff.data()?.rows ?? []; track r.key) {
                        <tr [class.clickable]="group() === 'stack'" [class.sel]="r.key === stackKey()" (click)="selectStack(r.key)" (keydown.enter)="selectStack(r.key)" [attr.tabindex]="group() === 'stack' ? 0 : null">
                          <td class="mono ellipsis" [title]="r.label">{{ r.label }}</td>
                          <td class="num" [class.up]="r.deltaBytes > 0" [class.down]="r.deltaBytes < 0">{{ delta(r.deltaBytes) }}</td>
                          <td class="num">{{ countDelta(r.deltaCount) }}</td>
                          <td class="num">{{ bytes(r.bytes) }}</td>
                          <td class="num">{{ count(r.stacks) }}</td>
                        </tr>
                      }
                    </tbody>
                  </table>
                </imt-analysis-state>
              </div>
              @if (group() === 'stack' && stackKey() !== null) {
                <div class="side" tabindex="0" role="region" aria-label="Allocation stack" i18n-aria-label="@@umdh.stack.region">
                  <imt-analysis-state [loading]="stack.loading()" [error]="stack.error()" [retryable]="stack.retryable()" [hasData]="!!stack.data()" (retry)="stack.retry()">
                    @if (stack.data(); as s) {
                      <table class="grid" data-testid="umdh-stack" aria-label="Allocation stack" i18n-aria-label="@@umdh.stack.label">
                        <thead><tr><th class="num">#</th><th i18n="@@umdh.stack.frame">Frame</th></tr></thead>
                        <tbody>
                          @for (f of s.frames; track f.index) {
                            <tr [class.framework]="f.framework" [class.culprit]="f.culprit">
                              <td class="num">{{ f.index }}</td>
                              <td class="mono">{{ f.label }}@if (f.culprit) { <span class="tag culprit" i18n="@@umdh.stack.culprit">allocated here</span> }</td>
                            </tr>
                          }
                        </tbody>
                      </table>
                    }
                  </imt-analysis-state>
                </div>
              }
            </div>
          }
          @case ('tree') {
            <imt-analysis-state [loading]="treeLoading()" [error]="treeError()" [hasData]="tree().length > 0" [empty]="!treeLoading() && tree().length === 0" emptyText="No allocation stacks." i18n-emptyText="@@umdh.tree.empty" (retry)="loadRoots()">
              <div class="body">
                <cdk-tree #cdkTree [dataSource]="tree()" [levelAccessor]="level" [trackBy]="trackBy" [expansionKey]="track" class="tree" aria-label="Growing stacks, outermost caller first" i18n-aria-label="@@umdh.tree.label" data-testid="umdh-tree">
                  <cdk-tree-node *cdkTreeNodeDef="let item" [isExpandable]="item.node.hasChildren" (expandedChange)="expanded(item, $event)" [cdkTreeNodeTypeaheadLabel]="item.node.label" class="node" [class.framework]="item.node.framework" [style.padding-left.px]="8 + item.level * 16">
                    @if (item.node.hasChildren) {
                      <button type="button" class="twisty" cdkTreeNodeToggle tabindex="-1" [attr.aria-label]="toggleLabel(item)">{{ cdkTree.isExpanded(item) ? '▾' : '▸' }}</button>
                    } @else {
                      <span class="twisty" aria-hidden="true"></span>
                    }
                    <span class="mono ellipsis grow">{{ item.node.label }}</span>
                    <span class="num" [class.up]="item.node.deltaBytes > 0" [class.down]="item.node.deltaBytes < 0">{{ delta(item.node.deltaBytes) }}</span>
                  </cdk-tree-node>
                </cdk-tree>
              </div>
            </imt-analysis-state>
          }
        }
      </div>
    } @else {
      <imt-no-dump text="Open UMDH snapshots (umdh -p:PID -f:file) or a UMDH diff to find what leaks." i18n-text="@@umdh.none" action="Open UMDH snapshots…" i18n-action="@@umdh.open" (open)="store.pickAndOpen('umdh')" />
    }
  `,
  styles: [
    DUMPS_STYLES,
    `
      .panel { flex: 1; min-height: 0; display: flex; flex-direction: column; }
      .split { flex: 1; min-height: 0; display: flex; }
      .main { flex: 1; min-width: 0; overflow: auto; padding: 8px 12px; }
      .side { width: 45%; min-width: 240px; border-left: 1px solid var(--border); overflow: auto; padding: 8px 12px; }
      .group { display: inline-flex; gap: 10px; }
      .tree { display: block; }
      .node { display: flex; align-items: center; gap: 6px; padding: 2px 8px; border-bottom: 1px solid var(--border); }
      .node:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
      .node.framework .mono { color: var(--muted); }
      .twisty { all: unset; width: 14px; text-align: center; cursor: pointer; color: var(--muted); }
    `,
  ],
})
export class UmdhView {
  protected readonly store = inject(DumpsStore);
  private readonly backend = inject(DumpsBackend);
  private readonly explainer = inject(DumpsExplainer);
  private readonly cdkTree = viewChild<CdkTree<TreeItem, string>>('cdkTree');

  protected readonly tabs: { id: 'findings' | 'growth' | 'tree'; label: string }[] = [
    { id: 'findings', label: $localize`:@@umdh.tab.findings:Findings` },
    { id: 'growth', label: $localize`:@@umdh.tab.growth:Growth` },
    { id: 'tree', label: $localize`:@@umdh.tab.tree:Stack tree` },
  ];
  protected readonly groups: { id: UmdhGroup; label: string }[] = [
    { id: 'stack', label: $localize`:@@umdh.group.stack:Stack` },
    { id: 'module', label: $localize`:@@umdh.group.module:Module` },
    { id: 'function', label: $localize`:@@umdh.group.function:Function` },
  ];
  readonly tab = signal<'findings' | 'growth' | 'tree'>('findings');
  readonly group = signal<UmdhGroup>('stack');
  readonly filter = signal('');
  readonly stackKey = signal<string | null>(null);
  /** A stack to select once the growth table answered (a finding's evidence). */
  private pending: string | null = null;

  readonly findings = new AsyncResource<DumpFinding[]>();
  readonly diff = new AsyncResource<UmdhDiffPage>();
  readonly stack = new AsyncResource<UmdhStackDto>();
  readonly tree = signal<TreeItem[]>([]);
  readonly treeLoading = signal(false);
  readonly treeError = signal<string | null>(null);

  protected readonly target = computed<DumpDocSummary | null>(() => this.store.umdh().find((d) => d.id === this.store.umdhTarget()) ?? null);
  protected readonly snapshots = computed(() => this.store.umdh().filter((d) => d.kind === 'umdhSnapshot'));
  private readonly pair = computed(() => ({ target: this.store.umdhTarget(), base: this.store.umdhBase() }));
  protected readonly groupLabel = computed(() => this.groups.find((g) => g.id === this.group())!.label);
  protected readonly level = (i: TreeItem) => i.level;
  protected readonly track = (i: TreeItem) => i.path.join('\n');
  protected readonly trackBy = (_: number, i: TreeItem) => this.track(i);

  constructor() {
    effect(() => {
      // Findings over every open snapshot of the series (steady growth needs three), or the diff log alone.
      const t = this.target();
      const ids = t?.kind === 'umdhDiff' ? [t.id] : this.snapshots().map((d) => d.id);
      untracked(() => (t ? void this.findings.load(() => this.backend.findings({ ids })) : this.findings.reset()));
    });
    effect(() => {
      const { target, base } = this.pair();
      const group = this.group();
      const filter = this.filter().trim() || null;
      untracked(() => {
        this.stackKey.set(this.pending);
        this.pending = null;
        if (target === null) return this.diff.reset();
        void this.diff.load(() => this.backend.umdhDiff({ base, target, group, offset: 0, limit: PAGE, filter }));
      });
    });
    effect(() => {
      const { target, base } = this.pair();
      const key = this.stackKey();
      untracked(() => (target !== null && key !== null ? void this.stack.load(() => this.backend.umdhStack({ base, target, key })) : this.stack.reset()));
    });
    effect(() => {
      this.pair();
      if (this.tab() !== 'tree') return;
      untracked(() => void this.loadRoots());
    });
  }

  protected value(e: Event): string {
    return (e.target as HTMLInputElement).value;
  }

  protected setTarget(e: Event): void {
    const id = Number((e.target as HTMLSelectElement).value);
    const doc = this.store.umdh().find((d) => d.id === id);
    this.store.setUmdh(id, doc?.kind === 'umdhSnapshot' ? this.store.umdhBase() : null);
  }

  protected setBase(e: Event): void {
    const v = (e.target as HTMLSelectElement).value;
    this.store.setUmdh(this.store.umdhTarget(), v === '' ? null : Number(v));
  }

  protected closeTarget(): void {
    const t = this.store.umdhTarget();
    if (t !== null) void this.store.close(t);
  }

  selectStack(key: string): void {
    if (this.group() === 'stack') this.stackKey.set(key);
  }

  protected showStack(f: DumpFinding): void {
    if (f.evidence.stack === null) return;
    this.tab.set('growth');
    if (this.group() === 'stack') {
      this.stackKey.set(f.evidence.stack);
    } else {
      // Changing the grouping reloads the table, which then selects it.
      this.pending = f.evidence.stack;
      this.group.set('stack');
    }
  }

  async loadRoots(): Promise<void> {
    const { target, base } = this.pair();
    if (target === null) {
      this.tree.set([]);
      return;
    }
    this.treeLoading.set(true);
    this.treeError.set(null);
    try {
      const roots = await this.backend.umdhTree({ base, target, path: [], limit: TREE_LIMIT });
      this.tree.set(roots.map((node) => ({ node, path: [node.key], level: 0 })));
    } catch (e) {
      this.treeError.set(errorText(e));
    } finally {
      this.treeLoading.set(false);
    }
  }

  /** Loads a node's children the first time it opens. */
  async expanded(item: TreeItem, open: boolean): Promise<void> {
    if (!open) return;
    const next = this.tree()[this.tree().findIndex((i) => this.track(i) === this.track(item)) + 1];
    if (next && next.level > item.level) return;
    const { target, base } = this.pair();
    if (target === null) return;
    try {
      const kids = await this.backend.umdhTree({ base, target, path: item.path, limit: TREE_LIMIT });
      this.tree.set(insertChildren(this.tree(), item, kids));
    } catch (e) {
      this.treeError.set(errorText(e));
    }
  }

  protected toggleLabel(item: TreeItem): string {
    const label = item.node.label;
    return this.cdkTree()?.isExpanded(item) ? $localize`:@@umdh.tree.collapse:Collapse ${label}:frame:` : $localize`:@@umdh.tree.expand:Expand ${label}:frame:`;
  }

  protected async explain(): Promise<void> {
    const t = this.target();
    if (!t) return;
    const ids = t.kind === 'umdhDiff' ? [t.id] : this.snapshots().map((d) => d.id);
    try {
      await this.explainer.explain(ids);
    } catch (e) {
      this.findings.error.set(errorText(e));
    }
  }

  protected readonly bytes = formatBytes;
  protected readonly delta = formatDelta;
  protected readonly count = formatCount;
  protected readonly countDelta = formatCountDelta;
}
