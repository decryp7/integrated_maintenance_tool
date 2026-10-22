import { afterNextRender, ChangeDetectionStrategy, Component, computed, DestroyRef, ElementRef, inject, Injector, signal, viewChildren } from '@angular/core';
import { RovingFocusNavigation } from '../shared/list-navigation';
import { FilterOp, LEVELS, Level } from '../core/api.types';
import { formatCompact, formatDateTime, formatDuration, tzLabel } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { LevelCountsStore } from '../core/workspace/level-counts.store';
import { ClickOutside } from '../shared/click-outside';
import { LEVEL_CODES, LEVEL_VARS } from '../shared/colors';
import { Icon } from '../shared/icon';
import { Switch } from '../shared/switch';
import { TimelineZoom } from '../timeline/zoom';
import { QueryField, QueryStatus } from '../analysis/query-field';
import { SearchField } from '../analysis/search-field';

/** The filter bar folds its secondary controls into a menu when the bar is narrower than this, px (H1). */
export const COMPACT_WIDTH_PX = 860;

const OP_LABELS: Record<FilterOp, string> = {
  eq: '=',
  ne: '≠',
  contains: '~',
  notContains: '!~',
  matches: $localize`:Filter operator|@@workspace.filterBar.op.matches:matches`,
  exists: $localize`:Filter operator|@@workspace.filterBar.op.exists:exists`,
  missing: $localize`:Filter operator|@@workspace.filterBar.op.missing:missing`,
  gt: '>',
  lt: '<',
};

@Component({
  selector: 'imt-filter-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, ClickOutside, Switch, QueryField, QueryStatus],
  template: `
    <div class="row">
      <imt-query-field #qf />

      <!-- One tab stop: the arrow keys move between the chips (roving tabindex). -->
      <div class="levels" role="toolbar" aria-label="Levels" i18n-aria-label="@@workspace.filterBar.levels" (keydown)="onLevelsKey($event)">
        @for (l of levels; track l; let i = $index) {
          <button #lvl class="lvl" [attr.data-level]="l" [attr.tabindex]="i === levelFocus() ? 0 : -1" (focus)="levelFocused(i)" [class.on]="levelOn(l)" [class.none]="levelMatching(l) === 0" (click)="store.toggleLevel(l)" [attr.aria-pressed]="levelOn(l)" [title]="levelTitle(l)">
            <span class="dot" [style.background]="levelVar(l)"></span>
            <span class="mono code">{{ code(l) }}</span>
            <span class="mono faint">{{ levelCount(l) }}</span>
          </button>
        }
      </div>

      <button class="btn tall range" [class.fold]="compact()" (click)="store.hasRange() && zoom.fit()" [title]="rangeLabel() + ' · ' + rangeTitle()">
        <span class="mono ends"><span class="end">{{ rangeEnds()[0] }}</span>@if (rangeEnds()[1]) {<span> → </span><span class="end">{{ rangeEnds()[1] }}</span>}</span>
        @if (store.hasRange()) {
          <span class="faint">({{ rangeSpan() }})</span>
          <imt-icon name="zoomOut" [size]="13" />
        }
      </button>
      <button class="tz" [class.fold]="compact()" (click)="store.setTimezone(store.timezone() === 'utc' ? 'local' : 'utc')" title="Toggle UTC / local time" i18n-title="@@workspace.filterBar.toggleTz">{{ tz() }}</button>
      <div class="anchor" (imtClickOutside)="addOpen.set(false)">
        <button class="btn dashed pill" (click)="addOpen.set(!addOpen())" [attr.aria-expanded]="addOpen()" i18n="@@workspace.filterBar.addFilter">+ Add filter</button>
        @if (addOpen()) {
          <form class="popover add-pop" (submit)="$event.preventDefault(); addFilter()">
            <label class="lbl"><ng-container i18n="@@workspace.filterBar.field">Field</ng-container>
              <input class="input mono" list="imt-field-names" [value]="newField()" (input)="newField.set($any($event.target).value)" placeholder="SourceContext" required />
            </label>
            <datalist id="imt-field-names">
              @for (f of allFields(); track f) { <option [value]="f"></option> }
            </datalist>
            <label class="lbl"><ng-container i18n="@@workspace.filterBar.operator">Operator</ng-container>
              <select class="select" [value]="newOp()" (change)="newOp.set($any($event.target).value)">
                @for (op of ops; track op) { <option [value]="op" [selected]="op === newOp()">{{ opLabel(op) }} ({{ op }})</option> }
              </select>
            </label>
            @if (newOp() !== 'exists' && newOp() !== 'missing') {
              <label class="lbl"><ng-container i18n="@@workspace.filterBar.value">Value</ng-container>
                <input class="input mono" [value]="newValue()" (input)="newValue.set($any($event.target).value)" />
              </label>
            }
            <div class="pop-actions">
              <button type="button" class="btn ghost" (click)="addOpen.set(false)" i18n="@@workspace.filterBar.cancel">Cancel</button>
              <button type="submit" class="btn primary" [disabled]="!newField()" i18n="@@workspace.filterBar.add">Add</button>
            </div>
          </form>
        }
      </div>

      @if (hasAnyFilter()) {
        <button class="btn ghost pill" [class.fold]="compact()" (click)="store.clearQuery()" i18n="@@workspace.filterBar.clearAll">Clear all</button>
      }
      <span class="spacer"></span>
      @if (compact()) {
        <!-- Narrow window (H1): what does not fit folds into this menu; the search and the levels always stay. -->
        <div class="anchor" (imtClickOutside)="moreOpen.set(false)" (keydown.escape)="moreOpen.set(false)">
          <button class="btn tall" data-testid="filter-more" aria-haspopup="menu" [attr.aria-expanded]="moreOpen()" aria-label="More filter controls" i18n-aria-label="@@workspace.filterBar.more" title="More filter controls" i18n-title="@@workspace.filterBar.moreTitle" (click)="moreOpen.set(!moreOpen())">
            <imt-icon name="more" [size]="14" />
          </button>
          @if (moreOpen()) {
            <div class="popover more-pop" role="menu" aria-label="More filter controls" i18n-aria-label="@@workspace.filterBar.more">
              <button type="button" role="menuitem" class="opt" [disabled]="!store.hasRange()" (click)="moreOpen.set(false); zoom.fit()">{{ rangeLabel() }}</button>
              <button type="button" role="menuitem" class="opt" (click)="moreOpen.set(false); store.setTimezone(store.timezone() === 'utc' ? 'local' : 'utc')" i18n="@@workspace.filterBar.moreTz">Time zone: {{ tz() }}</button>
              @if (hasAnyFilter()) {
                <button type="button" role="menuitem" class="opt" (click)="moreOpen.set(false); store.clearQuery()" i18n="@@workspace.filterBar.clearAll">Clear all</button>
              }
            </div>
          }
        </div>
      }
      <div class="anchor" (imtClickOutside)="optionsOpen.set(false)" (keydown.escape)="optionsOpen.set(false)">
        <button class="btn tall" data-testid="view-options" aria-haspopup="dialog" [attr.aria-expanded]="optionsOpen()" aria-controls="imt-view-options" (click)="optionsOpen.set(!optionsOpen())">
          <imt-icon name="group" [size]="14" />
          <span [class.fold]="compact()" i18n="@@workspace.filterBar.viewOptions">View options</span>
          <imt-icon name="chevronDown" [size]="12" />
        </button>
        @if (optionsOpen()) {
          <div class="popover options-pop" id="imt-view-options" role="dialog" aria-label="View options" i18n-aria-label="@@workspace.filterBar.viewOptionsAria">
            <div class="opt-section">
              <div class="opt-head"><span class="muted" i18n="@@workspace.filterBar.groupLanesBy">Group events into timeline lanes by</span> <span class="mono">{{ store.effectiveGroupBy().join(' · ') || '—' }}</span></div>
              <div class="faint small" i18n="@@workspace.filterBar.groupLanesHelp">Events with equal values in these fields form one lane.</div>
              <div class="chosen">
                @for (g of store.effectiveGroupBy(); track g; let i = $index) {
                  <span class="chip"><span class="mono">{{ i + 1 }}. {{ g }}</span>
                    <button class="x" [attr.aria-label]="removeLabel(g)" (click)="removeGroupField(g)" [attr.title]="removeLabel(g)">×</button></span>
                }
              </div>
              <div class="opts" role="group" aria-label="Add a field to the group-by" i18n-aria-label="@@workspace.filterBar.groupCandidates">
                @for (f of groupCandidates(); track f) {
                  <button class="opt mono" (click)="addGroupField(f)">+ {{ f }}</button>
                }
              </div>
              <div class="pop-actions">
                <button class="btn ghost" (click)="store.setGroupBy([])" i18n="@@workspace.filterBar.profileDefault">Use profile default</button>
              </div>
            </div>
            <div class="opt-section switches">
              <imt-switch class="sw" size="small" [checked]="store.markedOnly()" (checkedChange)="store.setMarkedOnly($event)" i18n="@@workspace.filterBar.markedOnly">Marked only</imt-switch>
              <imt-switch class="sw" size="small" [checked]="store.alwaysShowLifecycle()" (checkedChange)="store.setAlwaysShowLifecycle($event)" i18n="@@workspace.filterBar.alwaysLifecycle">Always show start / stop</imt-switch>
            </div>
          </div>
        }
      </div>
    </div>
    <imt-query-status (pick)="qf.useSuggestion($event)" />

    @if (q().filters.length || q().groups.length || field.hiddenText()) {
    <div class="row" role="group" aria-label="Filters" i18n-aria-label="@@workspace.filterBar.filters">
      @if (field.hiddenText(); as text) {
        <span class="chip" data-testid="hidden-search">
          <span class="muted" i18n="A message search that applies besides the query|@@workspace.filterBar.alsoSearch">also search</span>
          <span class="mono">{{ text }}</span>
          <button class="x" [attr.aria-label]="removeSearchLabel" (click)="field.dropHiddenText()" [attr.title]="removeSearchLabel">×</button>
        </span>
      }
      @for (f of q().filters; track $index; let i = $index) {
        <span class="chip">
          <span class="mono field">{{ f.field }}</span>
          <span class="muted">{{ opLabel(f.op) }}</span>
          @if (f.op !== 'exists' && f.op !== 'missing') { <span class="mono">{{ f.value }}</span> }
          <button class="x" [attr.aria-label]="removeFilterLabel(f.field)" (click)="store.removeFilter(i)" [attr.title]="removeFilterLabel(f.field)">×</button>
        </span>
      }
      @for (g of q().groups; track g) {
        <span class="chip">
          <span class="muted" i18n="@@workspace.filterBar.instance">instance</span>
          <span class="mono">{{ g }}</span>
          <button class="x" [attr.aria-label]="removeInstanceLabel(g)" (click)="store.toggleGroup(g)" [attr.title]="removeInstanceLabel(g)">×</button>
        </span>
      }

    </div>
    }
  `,
  styles: `
    :host { flex: none; display: flex; flex-direction: column; gap: 8px; padding: 10px 14px; border-bottom: 1px solid var(--border); background: var(--bg-1); }
    .row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
    .row:first-child { flex-wrap: nowrap; }
    .levels { flex: none; display: flex; height: 30px; border: 1px solid var(--border-2); border-radius: 6px; overflow: hidden; }
    .lvl { height: 30px; display: flex; align-items: center; gap: 6px; padding: 0 9px; border: 0; border-right: 1px solid var(--border-2); background: var(--bg-2); color: var(--faint); font-size: var(--fs-sm); cursor: pointer; }
    .lvl:last-child { border-right: 0; }
    .lvl.on { background: var(--bg-active); color: var(--text); }
    .lvl .code, .lvl .faint { font-size: var(--fs-xs); }
    .tall { height: 30px; flex: none; }
    /* The range shrinks (with an ellipsis) before anything else in the one-row bar. */
    .range { flex: 0 1 auto; min-width: 80px; overflow: hidden; }
    .range .ends { display: flex; gap: 4px; min-width: 0; overflow: hidden; white-space: nowrap; font-size: var(--fs-xs); }
    .range .end { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
    /* A chip that would add nothing ("0/403") keeps readable text (no opacity: AA contrast); only its dot fades. */
    .lvl.none .dot { opacity: 0.35; }
    .pill { height: 24px; border-radius: 12px; flex: none; }
    .tz { height: 22px; padding: 0 6px; border: 0; border-radius: 4px; background: var(--bg-active); color: var(--muted); font-size: var(--fs-xs); cursor: pointer; }
    .anchor { position: relative; }
    .fold { display: none; }
    .more-pop { top: 34px; right: 0; min-width: 220px; display: flex; flex-direction: column; gap: 2px; }
    .chip .field { color: var(--chip-text); }
    .x { border: 0; background: transparent; color: var(--faint); cursor: pointer; padding: 0 0 0 2px; font-size: var(--fs-lg); line-height: 1; }
    .x:hover { color: var(--text); }
    .spacer { flex: 1; }
    .options-pop { top: 34px; right: 0; width: 340px; display: flex; flex-direction: column; gap: 12px; }
    .opt-section { display: flex; flex-direction: column; gap: 8px; }
    .opt-head { display: flex; flex-wrap: wrap; gap: 6px; align-items: baseline; font-size: var(--fs-sm); }
    .switches { border-top: 1px solid var(--border); padding-top: 10px; }
    .sw { color: var(--muted); }
    .chosen { display: flex; flex-wrap: wrap; gap: 6px; }
    .opts { max-height: 220px; overflow: auto; display: flex; flex-direction: column; }
    /* flex: none: in the scrolling column the rows would otherwise shrink below the 24 px target size. */
    .opt { flex: none; white-space: nowrap; text-align: left; min-height: 24px; border: 0; border-radius: 4px; background: transparent; color: var(--text-2); cursor: pointer; font-size: var(--fs-sm); padding: 0 6px; }
    .opt:hover { background: var(--bg-hover); }
    .pop-actions { display: flex; justify-content: flex-end; gap: 6px; }
    .add-pop { top: 30px; left: 0; width: 300px; display: flex; flex-direction: column; gap: 8px; }
    .lbl { display: flex; flex-direction: column; gap: 4px; color: var(--muted); font-size: var(--fs-xs); }
    .small { font-size: var(--fs-xs); }
  `,
})
export class FilterBar {
  protected readonly store = inject(WorkspaceStore);
  protected readonly field = inject(SearchField);
  private readonly levelCounts = inject(LevelCountsStore);
  protected readonly removeSearchLabel = $localize`:@@workspace.filterBar.removeAlsoSearch:Remove the message search`;
  protected readonly zoom = inject(TimelineZoom);
  protected readonly levels = LEVELS;
  protected readonly ops: FilterOp[] = ['eq', 'ne', 'contains', 'notContains', 'matches', 'exists', 'missing', 'gt', 'lt'];
  protected readonly optionsOpen = signal(false);
  protected readonly moreOpen = signal(false);
  /** The window is narrow (the bar is narrower than `COMPACT_WIDTH_PX`): the range, time zone and Clear all fold into the ⋯ menu. */
  protected readonly compact = signal(false);
  protected readonly addOpen = signal(false);
  protected readonly newField = signal('');
  protected readonly newOp = signal<FilterOp>('eq');
  protected readonly newValue = signal('');
  private readonly levelButtons = viewChildren<ElementRef<HTMLButtonElement>>('lvl');
  /** The chip that holds the levels' one tab stop. */
  protected readonly levelFocus = signal(0);
  private readonly levelNav = new RovingFocusNavigation(signal(LEVELS.length), inject(Injector), (i) => this.levelButtons()[i]?.nativeElement.focus(), { wrap: true, homeAndEnd: true });

  protected readonly q = this.store.query;
  protected readonly rangeTitle = computed(() =>
    this.store.hasRange()
      ? $localize`:@@workspace.filterBar.resetRange:Reset to the full range`
      : $localize`:@@workspace.filterBar.dragToZoom:Drag on the timeline to zoom`,
  );
  protected readonly tz = computed(() => tzLabel(this.store.timezone()));
  protected readonly allFields = computed(() => ['@level', '@message', '@exception', '@template', '@source', ...this.store.fieldNames()]);
  protected readonly groupCandidates = computed(() => {
    const chosen = new Set(this.store.effectiveGroupBy());
    return ['@source', ...this.store.fieldNames()].filter((f) => !chosen.has(f));
  });
  protected readonly hasAnyFilter = computed(() => {
    const q = this.q();
    return !!(q.text || q.ql || q.levels.length || q.filters.length || q.groups.length || q.markers.length || q.from != null || q.to != null);
  });
  /** Both ends of the range; each truncates on its own, so a narrow bar still shows where the range starts and ends. */
  protected readonly rangeEnds = computed<[string, string]>(() => {
    const tz = this.store.timezone();
    const q = this.q();
    const s = this.store.summary();
    if (q.from == null && q.to == null && s?.from == null) return [$localize`:@@workspace.filterBar.allTime:All time`, ''];
    return [formatDateTime(q.from ?? s?.from ?? null, tz), formatDateTime(q.to ?? s?.to ?? null, tz)];
  });
  /** The range as one line, for the button's tooltip. */
  protected readonly rangeLabel = computed(() => this.rangeEnds().filter(Boolean).join(' → '));
  protected readonly rangeSpan = computed(() => {
    const q = this.q();
    return q.from != null && q.to != null ? formatDuration(q.to - q.from) : '';
  });

  constructor() {
    const host = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;
    const destroyRef = inject(DestroyRef);
    afterNextRender(() => {
      if (typeof ResizeObserver !== 'function') return;
      const observer = new ResizeObserver((entries) => this.compact.set((entries[0]?.contentRect.width ?? Infinity) < COMPACT_WIDTH_PX));
      observer.observe(host);
      destroyRef.onDestroy(() => observer.disconnect());
    });
  }

  protected levelFocused(i: number): void {
    this.levelFocus.set(i);
    this.levelNav.sync(i);
  }

  /** Left/Right (and Home/End) move between the level chips; the toolbar is horizontal. */
  protected onLevelsKey(e: KeyboardEvent): void {
    const key = e.key === 'ArrowRight' ? 'ArrowDown' : e.key === 'ArrowLeft' ? 'ArrowUp' : e.key === 'Home' || e.key === 'End' ? e.key : null;
    if (!key) return;
    if (this.levelNav.handle(new KeyboardEvent('keydown', { key }))) e.preventDefault();
  }

  protected levelOn(l: Level): boolean {
    return this.q().levels.includes(l);
  }
  protected levelVar(l: Level): string {
    return LEVEL_VARS[l];
  }
  protected code(l: Level): string {
    return LEVEL_CODES[l];
  }
  /** Events of level `l` matching the other filters; null while nothing else narrows the result. */
  protected levelMatching(l: Level): number | null {
    return this.levelCounts.matching()?.[l] ?? null;
  }
  /** "403", or "12/403" when the other filters narrow the result. */
  protected levelCount(l: Level): string {
    const total = formatCompact(this.store.summary()?.levels[l] ?? 0);
    const matching = this.levelMatching(l);
    return matching === null ? total : `${formatCompact(matching)}/${total}`;
  }
  protected levelTitle(l: Level): string {
    const matching = this.levelMatching(l);
    if (matching === null) return l;
    const total = this.store.summary()?.levels[l] ?? 0;
    return $localize`:@@workspace.filterBar.levelMatching:${l}:level:: ${matching}:matching: of ${total}:total: match the other filters`;
  }
  protected opLabel(op: FilterOp): string {
    return OP_LABELS[op];
  }

  protected removeLabel(name: string): string {
    return $localize`:@@workspace.filterBar.removeAria:Remove ${name}:name:`;
  }
  protected removeFilterLabel(field: string): string {
    return $localize`:@@workspace.filterBar.removeFilterAria:Remove filter ${field}:field:`;
  }
  protected removeInstanceLabel(group: string): string {
    return $localize`:@@workspace.filterBar.removeInstanceAria:Remove instance filter ${group}:group:`;
  }

  protected addGroupField(f: string): void {
    this.store.setGroupBy([...this.store.effectiveGroupBy(), f]);
  }
  protected removeGroupField(f: string): void {
    this.store.setGroupBy(this.store.effectiveGroupBy().filter((g) => g !== f));
  }

  protected addFilter(): void {
    const field = this.newField().trim();
    if (!field) return;
    const op = this.newOp();
    this.store.addFilter(op === 'exists' || op === 'missing' ? { field, op } : { field, op, value: this.newValue() });
    this.addOpen.set(false);
    this.newValue.set('');
  }
}
