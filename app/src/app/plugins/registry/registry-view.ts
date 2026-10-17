import { ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, signal, untracked, viewChildren } from '@angular/core';
import { RecordView, SourceInfo } from '../../core/api.types';
import { ImtBackend } from '../../core/backend';
import { formatCount } from '../../core/format';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { ImtEvents } from '../../extensions/events';
import { Icon } from '../../shared/icon';
import { loadRegistryRows } from './registry-loader';
import { buildRegistryTree, expandableIds, flattenTree, FlatNode, REGISTRY_ROW_CAP, RegistryNode } from './registry-tree';

/** Trees up to this many nodes open fully; bigger ones open only down to the hives. */
const AUTO_EXPAND_LIMIT = 400;

/** Windows registry exports as an expandable tree: Hive → Key → values (with type badge and data). */
@Component({
  selector: 'imt-registry-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    <div class="bar">
      <span class="section-title">Registry</span>
      <input class="input filter" type="search" placeholder="Filter keys, values, data" aria-label="Filter registry" [value]="filter()" (input)="filter.set($any($event.target).value)" />
      <span class="spacer"></span>
      <button class="btn ghost" [disabled]="!selectedNode()" (click)="copyPath()" title="Copy the path of the selected key or value">
        <imt-icon name="copy" [size]="14" /> {{ copied() ? 'Copied' : 'Copy path' }}
      </button>
    </div>
    @if (truncated()) {
      <div class="notice" role="status">Showing the first {{ cap }} rows only; narrow the registry export to see the rest.</div>
    }
    @if (loading()) {
      <div class="empty-state">Loading registry…</div>
    } @else if (!rows().length) {
      <div class="empty-state">No registry data in the workspace.</div>
    } @else {
      <div class="tree" role="tree" aria-label="Registry" (keydown)="onKey($event)">
        @for (item of flat(); track item.node.id) {
          <div
            #row
            class="item"
            role="treeitem"
            [attr.aria-level]="item.level"
            [attr.aria-posinset]="item.posInSet"
            [attr.aria-setsize]="item.setSize"
            [attr.aria-expanded]="item.expandable ? item.expanded : null"
            [attr.aria-selected]="item.node.id === selected()"
            [attr.data-id]="item.node.id"
            [attr.tabindex]="item.node.id === focusId() ? 0 : -1"
            [class.sel]="item.node.id === selected()"
            [style.padding-left.px]="8 + (item.level - 1) * 16"
            (click)="activate(item)"
          >
            <span class="twisty" aria-hidden="true">
              @if (item.expandable) { <imt-icon [name]="item.expanded ? 'chevronDown' : 'chevronRight'" [size]="12" /> }
            </span>
            <imt-icon class="kind" [name]="item.node.kind === 'value' ? 'file' : 'folder'" [size]="13" />
            <span class="label mono ellipsis">{{ item.node.label }}</span>
            @if (item.node.deleted) {
              <span class="badge">DELETED KEY</span>
            }
            @if (item.node.valueType) {
              <span class="badge">{{ item.node.valueType }}</span>
            }
            @if (item.node.kind === 'value') {
              <span class="data mono ellipsis" [title]="item.node.data">{{ item.node.data }}</span>
            }
          </div>
        } @empty {
          <div class="faint pad">Nothing matches "{{ filter() }}".</div>
        }
      </div>
    }
  `,
  styles: `
    :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .bar { flex: none; display: flex; align-items: center; gap: 10px; padding: 6px 12px; border-bottom: 1px solid var(--border); }
    .filter { width: 280px; }
    .spacer { flex: 1; }
    .notice { flex: none; padding: 6px 12px; color: var(--warn-text); border-bottom: 1px solid var(--border); font-size: 12px; }
    .tree { flex: 1; min-height: 0; overflow: auto; padding: 4px 0; }
    .item { display: flex; align-items: center; gap: 6px; height: 26px; padding-right: 12px; cursor: default; color: var(--text-2); white-space: nowrap; }
    .item:hover { background: var(--bg-hover); }
    .item.sel { background: var(--bg-sel); color: var(--text); }
    .item:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
    .twisty { width: 14px; flex: none; display: inline-flex; }
    .kind { color: var(--faint); flex: none; }
    .label { flex: none; max-width: 40%; font-size: 12px; }
    .data { flex: 1; min-width: 0; font-size: 12px; color: var(--muted); }
    .pad { padding: 8px 14px; }
  `,
})
export class RegistryView {
  private readonly backend = inject(ImtBackend);
  private readonly events = inject(ImtEvents);
  private readonly ctx = inject(COMMAND_CONTEXT);
  private readonly rowEls = viewChildren<ElementRef<HTMLElement>>('row');

  protected readonly cap = formatCount(REGISTRY_ROW_CAP);
  protected readonly filter = signal('');
  protected readonly loading = signal(true);
  protected readonly rows = signal<readonly RecordView[]>([]);
  protected readonly truncated = signal(false);
  protected readonly expanded = signal<ReadonlySet<string>>(new Set());
  protected readonly selected = signal<string | null>(null);
  protected readonly focusId = signal<string | null>(null);
  protected readonly copied = signal(false);

  protected readonly tree = computed(() => buildRegistryTree(this.rows()));
  protected readonly flat = computed(() => flattenTree(this.tree(), this.expanded(), this.filter()));
  protected readonly selectedNode = computed<RegistryNode | null>(() => this.flat().find((f) => f.node.id === this.selected())?.node ?? null);

  private loadSeq = 0;

  constructor() {
    // Reload whenever the set of sources (or their contents) changes.
    effect(() => {
      const summary = this.events.workspace();
      const sources = (summary?.sources ?? []).filter((s) => s.importer === 'registry');
      untracked(() => void this.load(sources));
    });
    effect(() => {
      const rows = this.flat();
      const focus = this.focusId();
      if (!rows.length) return;
      if (!focus || !rows.some((r) => r.node.id === focus)) untracked(() => this.focusId.set(rows[0].node.id));
    });
  }

  private async load(sources: SourceInfo[]): Promise<void> {
    const seq = ++this.loadSeq;
    this.loading.set(true);
    try {
      const result = await loadRegistryRows(this.backend, sources);
      if (seq !== this.loadSeq) return;
      this.rows.set(result.rows);
      this.truncated.set(result.truncated);
      const ids = expandableIds(buildRegistryTree(result.rows));
      this.expanded.set(new Set(result.rows.length <= AUTO_EXPAND_LIMIT ? ids : ids.filter((id) => !id.includes('\\'))));
    } catch (e) {
      if (seq === this.loadSeq) this.ctx.notifyError(`Registry could not be read: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      if (seq === this.loadSeq) this.loading.set(false);
    }
  }

  protected activate(item: FlatNode): void {
    this.focusId.set(item.node.id);
    this.selected.set(item.node.id);
    if (item.expandable && item.node.kind !== 'value') this.toggle(item.node.id);
    if (item.node.record) this.ctx.selectRecord(item.node.record);
  }

  private toggle(id: string, open?: boolean): void {
    const next = new Set(this.expanded());
    const want = open ?? !next.has(id);
    if (want) next.add(id);
    else next.delete(id);
    this.expanded.set(next);
  }

  protected onKey(e: KeyboardEvent): void {
    const list = this.flat();
    const index = list.findIndex((f) => f.node.id === this.focusId());
    const current = list[index];
    if (!current) return;
    const move = (i: number) => {
      const target = list[Math.max(0, Math.min(list.length - 1, i))];
      this.focusId.set(target.node.id);
      queueMicrotask(() => this.rowEls().find((el) => el.nativeElement.dataset['id'] === target.node.id)?.nativeElement.focus());
    };
    switch (e.key) {
      case 'ArrowDown':
        move(index + 1);
        break;
      case 'ArrowUp':
        move(index - 1);
        break;
      case 'Home':
        move(0);
        break;
      case 'End':
        move(list.length - 1);
        break;
      case 'ArrowRight':
        if (current.expandable && !current.expanded) this.toggle(current.node.id, true);
        else if (current.expanded) move(index + 1);
        break;
      case 'ArrowLeft':
        if (current.expanded) this.toggle(current.node.id, false);
        else {
          const parent = this.parentIndex(list, index);
          if (parent >= 0) move(parent);
        }
        break;
      case 'Enter':
      case ' ':
        this.activate(current);
        break;
      default:
        return;
    }
    e.preventDefault();
  }

  private parentIndex(list: FlatNode[], index: number): number {
    for (let i = index - 1; i >= 0; i--) if (list[i].level < list[index].level) return i;
    return -1;
  }

  protected async copyPath(): Promise<void> {
    const node = this.selectedNode();
    if (!node) return;
    try {
      await navigator.clipboard.writeText(node.path);
      this.copied.set(true);
      setTimeout(() => this.copied.set(false), 1500);
    } catch {
      this.ctx.notifyError('Clipboard is not available.');
    }
  }
}
