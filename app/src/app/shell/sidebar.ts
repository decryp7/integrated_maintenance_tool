import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { Facet, FieldInfo, SourceInfo } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { formatCompact, formatCount, formatDateTime } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { ClickOutside } from '../shared/click-outside';
import { Icon } from '../shared/icon';

/** Last folder + file name, e.g. "web01/orders.clef". */
function shortPath(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).slice(-2).join('/');
}

@Component({
  selector: 'imt-sidebar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, ClickOutside],
  template: `
    <div class="head">
      <span class="section-title">Explorer</span>
      <span class="tools">
        <button class="icon-btn" aria-label="Open files" title="Open files" (click)="store.pickAndOpen(false)"><imt-icon name="file" [size]="14" /></button>
        <button class="icon-btn" aria-label="Open folder" title="Open folder" (click)="store.pickAndOpen(true)"><imt-icon name="folder" [size]="14" /></button>
        <button class="icon-btn" aria-label="Re-import all" title="Re-import all" (click)="store.reload()"><imt-icon name="refresh" [size]="14" /></button>
      </span>
    </div>

    <div class="sources">
      @for (s of sources(); track s.id) {
        <div class="src" [title]="sourceTitle(s)">
          <imt-icon name="file" [size]="13" class="faint" />
          <span class="ellipsis name">{{ short(s.path) }}</span>
          <span class="badge">{{ s.importer }}</span>
          <span class="mono count">{{ compact(s.records) }}</span>
          <button class="icon-btn rm" [attr.aria-label]="'Remove ' + s.name" (click)="store.removeSource(s.id)"><imt-icon name="close" [size]="12" /></button>
        </div>
        @if (s.error) {
          <div class="src-error">{{ s.error }}</div>
        }
      } @empty {
        <div class="empty">
          <p>No files open.</p>
          <div class="open-row">
            <button class="btn primary" (click)="store.pickAndOpen(true)">Open folder…</button>
            <button class="btn" (click)="store.pickAndOpen(false)">Open files…</button>
          </div>
          <p class="faint small">Or drop log files anywhere in the window. Any extension works: the format is detected from the content.</p>
        </div>
      }
      @for (u of unrecognized(); track u.path) {
        <div class="src dim" [title]="u.reason">
          <imt-icon name="file" [size]="13" />
          <span class="ellipsis name">{{ short(u.path) }}</span>
          <span class="badge">—</span>
        </div>
      }
    </div>

    <div class="divider"></div>

    <div class="head">
      <span class="section-title">Fields</span>
      <span class="faint small">coverage · distinct</span>
    </div>
    <input class="input filter" placeholder="Filter fields" aria-label="Filter fields" [value]="fieldFilter()" (input)="fieldFilter.set($any($event.target).value)" />
    <div class="fields">
      @for (f of fields(); track f.name) {
        <div class="field-wrap">
          <button class="field" [class.grouped]="isGrouped(f.name)" (click)="openFacet(f, $event)" [attr.aria-expanded]="facetField() === f.name">
            <span class="gmark" aria-hidden="true">{{ isGrouped(f.name) ? '⊞' : '' }}</span>
            <span class="mono fname ellipsis">{{ f.name }}</span>
            @if (f.derived) { <span class="badge" title="Extracted by the profile">fx</span> }
            <span class="cov" [title]="coverage(f) + '% of events'"><span [style.width.%]="coverage(f)"></span></span>
            <span class="mono faint distinct">{{ f.distinct >= 10000 ? '10k+' : compact(f.distinct) }}</span>
          </button>
        </div>
      }
    </div>
    @if (facetPos(); as pos) {
      <div class="popover facet" [style.top.px]="pos.top" [style.left.px]="pos.left" (imtClickOutside)="closeFacet()">
        <div class="facet-head">
          <span class="mono">{{ pos.field }}</span>
          <span class="faint">{{ facet()?.distinct ?? '…' }} values in current results</span>
        </div>
        <div class="facet-actions">
          <button class="btn" (click)="groupBy(pos.field, false)">Group by</button>
          <button class="btn" (click)="groupBy(pos.field, true)">Add to group</button>
          <button class="btn" (click)="store.addColumn(pos.field); closeFacet()">Add column</button>
        </div>
        <div class="values">
          @for (v of facet()?.values ?? []; track v.value) {
            <div class="value">
              <span class="mono ellipsis" [title]="v.value">{{ v.value || '(empty)' }}</span>
              <span class="bar"><span [style.width.%]="share(v.count)"></span></span>
              <span class="mono faint">{{ count(v.count) }}</span>
              <button class="icon-btn" title="Only this value" [attr.aria-label]="'Filter ' + pos.field + ' equals ' + v.value" (click)="filter(pos.field, 'eq', v.value)"><imt-icon name="plus" [size]="12" /></button>
              <button class="icon-btn" title="Exclude this value" [attr.aria-label]="'Filter ' + pos.field + ' not equal ' + v.value" (click)="filter(pos.field, 'ne', v.value)"><imt-icon name="minus" [size]="12" /></button>
            </div>
          } @empty {
            <div class="faint">{{ facet() ? 'No values in the current results.' : 'Loading…' }}</div>
          }
        </div>
      </div>
    }
  `,
  styles: `
    :host { width: 272px; flex: none; display: flex; flex-direction: column; background: var(--bg-2); border-right: 1px solid var(--border); min-height: 0; }
    .head { height: 34px; flex: none; display: flex; align-items: center; justify-content: space-between; padding: 0 8px 0 12px; }
    .tools { display: flex; gap: 2px; }
    .small { font-size: 11px; }
    .sources { padding: 0 8px 4px; max-height: 40%; overflow: auto; flex: none; }
    .src { display: flex; align-items: center; gap: 8px; height: 26px; padding: 0 2px 0 6px; border-radius: 5px; color: var(--text-2); }
    .src:hover { background: var(--bg-hover); }
    .src .name { flex: 1; min-width: 0; }
    .src .count { font-size: 11px; color: var(--faint); width: 38px; text-align: right; }
    .src .rm { width: 20px; height: 20px; visibility: hidden; }
    .src:hover .rm, .src .rm:focus-visible { visibility: visible; }
    .src.dim { color: var(--faint); font-style: italic; }
    .src.dim imt-icon { opacity: 0.6; }
    .src-error { margin: 0 6px 4px 27px; font-size: 11px; color: var(--danger-text); }
    .empty { padding: 8px 6px; color: var(--muted); }
    .empty p { margin: 4px 0 10px; }
    .open-row { display: flex; gap: 6px; margin-bottom: 8px; }
    .divider { height: 1px; background: var(--border); margin: 6px 0; flex: none; }
    .filter { margin: 0 8px 6px; flex: none; }
    .fields { flex: 1; min-height: 0; overflow: auto; padding: 0 8px 8px; }
    .field-wrap { position: relative; }
    .field { width: 100%; display: flex; align-items: center; gap: 8px; height: 26px; padding: 0 6px; border: 0; border-radius: 5px; background: transparent; cursor: pointer; text-align: left; color: var(--text-2); }
    .field:hover { background: var(--bg-hover); }
    .field.grouped { background: var(--bg-active); }
    .gmark { width: 12px; color: var(--accent); font-size: 11px; }
    .fname { flex: 1; min-width: 0; font-size: 12px; }
    .cov { width: 40px; height: 4px; border-radius: 2px; background: var(--border-2); overflow: hidden; flex: none; }
    .cov span { display: block; height: 4px; background: var(--faint); border-radius: 2px; }
    .distinct { width: 34px; text-align: right; font-size: 11px; }
    .facet { position: fixed; width: 340px; display: flex; flex-direction: column; gap: 8px; max-height: 420px; }
    .facet-head { display: flex; justify-content: space-between; gap: 8px; }
    .facet-actions { display: flex; gap: 6px; flex-wrap: wrap; }
    .values { overflow: auto; display: flex; flex-direction: column; gap: 1px; }
    .value { display: grid; grid-template-columns: minmax(0, 1fr) 60px 48px 22px 22px; align-items: center; gap: 6px; min-height: 24px; }
    .value .mono { font-size: 11px; }
    .value .icon-btn { width: 22px; height: 22px; }
    .bar { height: 4px; background: var(--border-2); border-radius: 2px; overflow: hidden; }
    .bar span { display: block; height: 4px; background: var(--accent); border-radius: 2px; }
  `,
})
export class Sidebar {
  protected readonly store = inject(WorkspaceStore);
  private readonly backend = inject(ImtBackend);

  protected readonly compact = formatCompact;
  protected readonly count = formatCount;
  protected readonly short = shortPath;
  protected readonly fieldFilter = signal('');
  protected readonly facetField = signal<string | null>(null);
  protected readonly facet = signal<Facet | null>(null);
  protected readonly facetPos = signal<{ field: string; top: number; left: number } | null>(null);

  protected readonly sources = computed(() => this.store.summary()?.sources ?? []);
  protected readonly unrecognized = computed(() => this.store.summary()?.unrecognized ?? []);
  protected readonly fields = computed(() => {
    const needle = this.fieldFilter().toLowerCase();
    const all = this.store.summary()?.fields ?? [];
    return needle ? all.filter((f) => f.name.toLowerCase().includes(needle)) : all;
  });

  protected isGrouped(name: string): boolean {
    return this.store.effectiveGroupBy().includes(name);
  }

  protected coverage(f: FieldInfo): number {
    const total = this.store.summary()?.total ?? 0;
    return total ? Math.round((f.count / total) * 100) : 0;
  }

  protected share(n: number): number {
    const total = this.facet()?.total ?? 0;
    return total ? (n / total) * 100 : 0;
  }

  protected sourceTitle(s: SourceInfo): string {
    const tz = this.store.timezone();
    return `${s.path}\n${formatCount(s.records)} events · ${s.skipped} skipped\n${formatDateTime(s.from, tz)} → ${formatDateTime(s.to, tz)}`;
  }

  protected closeFacet(): void {
    this.facetField.set(null);
    this.facetPos.set(null);
  }

  protected async openFacet(f: FieldInfo, event: MouseEvent): Promise<void> {
    if (this.facetField() === f.name) {
      this.closeFacet();
      return;
    }
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const top = Math.min(rect.top, window.innerHeight - 440);
    this.facetPos.set({ field: f.name, top: Math.max(40, top), left: rect.right + 8 });
    this.facetField.set(f.name);
    this.facet.set(null);
    try {
      const facet = await this.backend.queryFacet({ query: this.store.effectiveQuery(), field: f.name, limit: 50 });
      if (this.facetField() === f.name) this.facet.set(facet);
    } catch (e) {
      this.store.setError(String(e));
    }
  }

  protected filter(field: string, op: 'eq' | 'ne', value: string): void {
    this.store.addFilter({ field, op, value });
    this.closeFacet();
  }

  protected groupBy(field: string, add: boolean): void {
    const current = this.store.effectiveGroupBy();
    this.store.setGroupBy(add ? [...current.filter((g) => g !== field), field] : [field]);
    this.closeFacet();
  }
}
