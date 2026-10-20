import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { AnalysisTargets } from '../analysis/analysis-targets';
import { FieldInfo } from '../core/api.types';
import { formatCompact } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { AdhocCharts } from '../metrics/adhoc-charts';
import { Icon } from '../shared/icon';
import { FacetPopover } from './facet-popover';
import { SidebarSections } from '../shell/sidebar-sections';

/** The workspace's fields with coverage and distinct counts; a field opens its facet popover or its statistics. */
@Component({
  selector: 'imt-fields-panel',
  host: { '[class.open]': 'sections.fieldsOpen()' },
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, FacetPopover],
  template: `
    <div class="head">
      <button type="button" class="toggle" data-testid="fields-toggle" aria-controls="imt-fields-body" [attr.aria-expanded]="sections.fieldsOpen()" (click)="sections.setFieldsOpen(!sections.fieldsOpen())">
        <imt-icon [name]="sections.fieldsOpen() ? 'chevronDown' : 'chevronRight'" [size]="12" />
        <span class="section-title" i18n="@@workspace.fields.title">Fields</span>
      </button>
      @if (sections.fieldsOpen()) {
        <span class="faint small" i18n="@@workspace.fields.legend">coverage · distinct</span>
      }
    </div>
    @if (sections.fieldsOpen()) {
    <div class="body" id="imt-fields-body">
    <input class="input filter" placeholder="Filter fields" i18n-placeholder="@@workspace.fields.filterPlaceholder" aria-label="Filter fields" i18n-aria-label="@@workspace.fields.filter" [value]="fieldFilter()" (input)="fieldFilter.set($any($event.target).value)" />
    <div class="fields">
      @for (f of fields(); track f.name) {
        <div class="field-wrap">
          <button class="field" [class.grouped]="isGrouped(f.name)" (click)="openFacet(f, $event)" [attr.aria-expanded]="facetField() === f.name">
            <span class="gmark" aria-hidden="true">{{ isGrouped(f.name) ? '⊞' : '' }}</span>
            <span class="mono fname ellipsis">{{ f.name }}</span>
            @if (f.derived) { <span class="badge" title="Extracted by the profile" i18n-title="@@workspace.fields.derived">fx</span> }
            <span class="cov" [title]="coverageTitle(f)"><span [style.width.%]="coverage(f)"></span></span>
            <span class="mono faint distinct">{{ f.distinct >= 10000 ? '10k+' : compact(f.distinct) }}</span>
          </button>
          <button class="icon-btn stats" type="button" [attr.aria-label]="statsLabel(f.name)" title="Statistics…" i18n-title="@@workspace.fields.stats" (click)="showStats(f.name)">
            <imt-icon name="chart" [size]="12" />
          </button>
        </div>
      }
    </div>
    </div>
    }
    @if (facetPos(); as pos) {
      <imt-facet-popover
        [field]="pos.field"
        [top]="pos.top"
        [left]="pos.left"
        [numeric]="store.numericFields().includes(pos.field)"
        (closed)="closeFacet()"
        (groupBy)="groupBy(pos.field, $event)"
        (addColumn)="store.addColumn(pos.field); closeFacet()"
        (chart)="chartField(pos.field)"
        (filter)="filter(pos.field, $event.op, $event.value)"
      />
    }
  `,
  styles: `
    :host { min-height: 0; display: flex; flex-direction: column; }
    :host(.open) { flex: 1; }
    .body { flex: 1; min-height: 0; display: flex; flex-direction: column; }
    .toggle { display: flex; align-items: center; gap: 4px; padding: 0; border: 0; background: transparent; color: inherit; cursor: pointer; }
    .toggle:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; border-radius: 4px; }
    .head { height: 34px; flex: none; display: flex; align-items: center; justify-content: space-between; padding: 0 8px 0 12px; }
    .small { font-size: 11px; }
    .filter { margin: 0 8px 6px; flex: none; }
    .fields { flex: 1; min-height: 0; overflow: auto; padding: 0 8px 8px; }
    .field-wrap { position: relative; display: flex; align-items: center; border-radius: 5px; }
    .field-wrap:hover { background: var(--bg-hover); }
    .field { flex: 1; min-width: 0; display: flex; align-items: center; gap: 8px; height: 26px; padding: 0 6px; border: 0; border-radius: 5px; background: transparent; cursor: pointer; text-align: left; color: var(--text-2); }
    .field.grouped { background: var(--bg-active); }
    .gmark { width: 12px; color: var(--accent); font-size: 11px; }
    .fname { flex: 1; min-width: 0; font-size: 12px; }
    .cov { width: 40px; height: 4px; border-radius: 2px; background: var(--border-2); overflow: hidden; flex: none; }
    .cov span { display: block; height: 4px; background: var(--faint); border-radius: 2px; }
    .distinct { width: 34px; text-align: right; font-size: 11px; }
    .stats { width: 20px; height: 20px; flex: none; visibility: hidden; }
    .field-wrap:hover .stats, .stats:focus-visible, .field:focus-visible + .stats { visibility: visible; }
  `,
})
export class FieldsPanel {
  protected readonly store = inject(WorkspaceStore);
  /** The section starts collapsed and remembers the choice. */
  protected readonly sections = inject(SidebarSections);
  private readonly adhocCharts = inject(AdhocCharts);
  private readonly targets = inject(AnalysisTargets);
  private readonly commands = inject(COMMAND_CONTEXT);

  protected readonly compact = formatCompact;
  protected readonly fieldFilter = signal('');
  protected readonly facetField = signal<string | null>(null);
  protected readonly facetPos = signal<{ field: string; top: number; left: number } | null>(null);

  protected readonly fields = computed(() => {
    const needle = this.fieldFilter().toLowerCase();
    const all = this.store.summary()?.fields ?? [];
    return needle ? all.filter((f) => f.name.toLowerCase().includes(needle)) : all;
  });

  protected isGrouped(name: string): boolean {
    return this.store.effectiveGroupBy().includes(name);
  }

  protected coverageTitle(f: FieldInfo): string {
    return $localize`:@@workspace.fields.coverage:${this.coverage(f)}:percent:% of events`;
  }

  protected statsLabel(name: string): string {
    return $localize`:@@workspace.fields.statsAria:Statistics for ${name}:name:`;
  }

  protected coverage(f: FieldInfo): number {
    const total = this.store.summary()?.total ?? 0;
    return total ? Math.round((f.count / total) * 100) : 0;
  }

  protected closeFacet(): void {
    this.facetField.set(null);
    this.facetPos.set(null);
  }

  protected openFacet(f: FieldInfo, event: MouseEvent): void {
    if (this.facetField() === f.name) {
      this.closeFacet();
      return;
    }
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const top = Math.min(rect.top, window.innerHeight - 480);
    this.facetPos.set({ field: f.name, top: Math.max(40, top), left: rect.right + 8 });
    this.facetField.set(f.name);
  }

  /** Opens the field statistics view for `field`. */
  protected showStats(field: string): void {
    this.closeFacet();
    this.targets.setField(field);
    this.commands.showView('field-stats');
  }

  protected filter(field: string, op: 'eq' | 'ne', value: string): void {
    this.store.addFilter({ field, op, value });
    this.closeFacet();
  }

  protected chartField(field: string): void {
    this.adhocCharts.addForField(field, this.store.effectiveGroupBy());
    this.commands.showView('metrics');
    this.closeFacet();
  }

  protected groupBy(field: string, add: boolean): void {
    const current = this.store.effectiveGroupBy();
    this.store.setGroupBy(add ? [...current.filter((g) => g !== field), field] : [field]);
    this.closeFacet();
  }
}
