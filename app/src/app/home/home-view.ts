import { CdkFixedSizeVirtualScroll, CdkVirtualForOf, CdkVirtualScrollViewport } from '@angular/cdk/scrolling';
import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { AiButton } from '../ai/ai-button';
import { AiLauncher } from '../ai/ai-launcher';
import { AnalysisState } from '../analysis/analysis-state';
import { CoverageNote } from '../analysis/coverage-note';
import { Finding, Level } from '../core/api.types';
import { formatCount, formatDateTime, formatDuration } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { COMMAND_CONTEXT, CommandRegistry } from '../extensions/command-registry';
import { EChart } from '../metrics/echart';
import { ChartTheme } from '../metrics/chart-theme';
import { RECENT_PROJECTS } from '../projects/recent-projects';
import { formatBytes } from '../shared/format/units';
import { Icon } from '../shared/icon';
import { PathText } from '../shared/path-text';
import { measuredWidth } from '../profiling/flame/measured-width';
import { HealthActions } from './health/health-actions';
import { ActionButton, actionButtons, healthStats } from './health/health-model';
import { bucketAt, bucketRange, feedItems, historyOption, sparklinePath, SPARK_H, SPARK_W } from './home-model';
import { HomePrefs } from './home-prefs';
import { HomeStore } from './home.store';

/** Height of one finding row in the virtualised list (fixed, so only the visible rows are rendered). */
const ROW_PX = 56;
/** Height of one finding row when the pane is narrow: the actions wrap below the text. */
const NARROW_ROW_PX = 116;
/** Pane width (px) below which findings stack their actions below the text. */
const NARROW_PANE_PX = 520;
/** Findings shown before "Show all". */
const FIRST_FINDINGS = 3;
/** Rows visible before the list scrolls inside itself. */
const VISIBLE_ROWS = 6;

/**
 * Home: the dashboard of everything opened. Live tiles (events, sources, errors and warnings with their trend, the
 * time span, exceptions, anomalies, the store), the history by level with its markers and anomalies (a click or a
 * drag across it sets the time range), the health check's findings (a bounded, virtualised list, most severe first,
 * each with its next step), the recent activity and quick actions. Every number is computed by the engine; this lays
 * them out. Each tile loads and fails on its own.
 */
@Component({
  selector: 'debrief-home-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[class.narrow]': 'narrow()' },
  imports: [AnalysisState, Icon, AiButton, CoverageNote, EChart, CdkVirtualScrollViewport, CdkFixedSizeVirtualScroll, CdkVirtualForOf, PathText],
  template: `
    <div class="bar">
      <p class="subtitle" data-testid="home-subtitle">@if (summary(); as s) { {{ span() }} } @else { <span i18n="@@home.view.subtitle">Everything you opened, at a glance</span> }</p>
      <span class="spacer"></span>
      <button class="icon-btn" type="button" data-testid="home-refresh" aria-label="Refresh" title="Refresh" i18n-aria-label="@@home.view.refresh" i18n-title="@@home.view.refreshTitle" (click)="home.refresh()" [disabled]="home.summaryLoading()"><debrief-icon name="refresh" [size]="14" /></button>
      <debrief-ai-button label="Root causes" i18n-label="@@home.view.rootCauses" hint="The local model reads the findings and exceptions and lists the likely root causes" i18n-hint="@@home.view.rootCausesHint" testId="ai-root-causes" (pressed)="ai.rootCauses()" />
    </div>
    <div class="scroll" tabindex="0" role="region" aria-label="Home dashboard" i18n-aria-label="@@home.view.region">
      <div class="actions" role="group" aria-label="Quick actions" i18n-aria-label="@@home.view.quickActions">
        <button class="btn primary" type="button" data-testid="home-open-files" (click)="run('file.openFiles')"><debrief-icon name="file" [size]="14" /> <span i18n="@@home.view.openFiles">Open files…</span></button>
        <button class="btn ghost" type="button" data-testid="home-open-folder" (click)="run('file.openFolder')"><debrief-icon name="folder" [size]="14" /> <span i18n="@@home.view.openFolder">Open folder…</span></button>
        @if (lastProject(); as last) {
          <button class="btn ghost" type="button" data-testid="home-open-last" [title]="last.name" (click)="run('project.openLast')"><debrief-icon name="home" [size]="14" /> <span i18n="@@home.view.openLast">Open last project</span></button>
        }
        <button class="btn ghost" type="button" data-testid="home-open-recent" (click)="run('project.recent')" i18n="@@home.view.openRecent">Recent projects…</button>
      </div>

      @if (!hasData()) {
        <p class="muted pad" data-testid="home-empty" i18n="@@home.view.empty">Open files or a folder to see the dashboard here.</p>
      } @else {
        <debrief-analysis-state [loading]="home.summaryLoading()" [error]="home.summaryError()" [retryable]="home.summaryRetryable()" [hasData]="!!summary()" [empty]="false" (retry)="home.retrySummary()">
          @if (summary(); as s) {
            <dl class="tiles" aria-label="Summary" i18n-aria-label="@@home.view.summary" data-testid="home-tiles">
              <div class="hero"><dt i18n="@@home.view.events">Events</dt><dd>{{ count(s.events) }}</dd></div>
              <div class="hero" [class.bad]="s.errors > 0">
                <dt i18n="@@home.view.errors">Errors</dt><dd>{{ count(s.errors) }}<button class="cover" type="button" data-testid="home-tile-errors" (click)="showLevels(['Error', 'Fatal'])" aria-label="Show the errors and fatal events" i18n-aria-label="@@home.view.showErrors"></button></dd>
                @if (errorSpark(); as d) { <svg class="spark bad" [attr.viewBox]="viewBox" preserveAspectRatio="none" aria-hidden="true"><path [attr.d]="d" /></svg> }
              </div>
              <div class="hero" [class.warn]="s.warnings > 0">
                <dt i18n="@@home.view.warnings">Warnings</dt><dd>{{ count(s.warnings) }}<button class="cover" type="button" data-testid="home-tile-warnings" (click)="showLevels(['Warning'])" aria-label="Show the warning events" i18n-aria-label="@@home.view.showWarnings"></button></dd>
                @if (warningSpark(); as d) { <svg class="spark warn" [attr.viewBox]="viewBox" preserveAspectRatio="none" aria-hidden="true"><path [attr.d]="d" /></svg> }
              </div>
              <div class="hero"><dt i18n="@@home.view.anomalies">Anomalies</dt><dd [attr.aria-busy]="!s.analysed">{{ s.analysed ? count(s.anomalyCount) : '…' }}<button class="cover" type="button" data-testid="home-tile-anomalies" (click)="show('anomalies')" aria-label="Open the Anomalies view" i18n-aria-label="@@home.view.showAnomalies"></button></dd></div>
              <div class="chip"><dt i18n="@@home.view.sources">Sources</dt><dd>{{ count(s.sources) }}</dd></div>
              <div class="chip" [class.bad]="crashes() > 0"><dt i18n="@@home.view.crashes">Crashes</dt><dd>{{ count(crashes()) }}</dd></div>
              <div class="chip"><dt i18n="@@home.view.exceptions">Exception groups</dt><dd [attr.aria-busy]="!s.analysed">{{ s.analysed ? count(s.exceptionGroups) : '…' }}<button class="cover" type="button" data-testid="home-tile-exceptions" (click)="show('exceptions')" aria-label="Open the Exceptions view" i18n-aria-label="@@home.view.showExceptions"></button></dd></div>
              @if (home.storeOverview.data(); as o) {
                <div class="chip"><dt i18n="@@home.view.storeSize">Store</dt><dd>{{ bytes(o.sizeBytes) }}</dd></div>
              }
              @if (home.memory.data(); as m) {
                <div class="chip"><dt i18n="@@home.view.memory">Memory</dt><dd>{{ bytes(m.worker?.rssBytes ?? m.duckUsedBytes) }}</dd></div>
              }
            </dl>
            <debrief-coverage-note [coverage]="s.coverage" />

            @if (option(); as o) {
              <section class="history" aria-labelledby="home-history">
                <h3 id="home-history"><span i18n="@@home.view.history">Events over time</span> <span class="faint hint" i18n="@@home.view.historyHint">Click a bar to show that period, or drag across the chart to select a range.</span></h3>
                <debrief-echart class="chart" data-testid="home-history" [options]="o" (plotClick)="selectBucket($event)" (brushed)="selectRange($event)" />
              </section>
            }
          }
        </debrief-analysis-state>

        <div class="pair">
        @if (stats(); as st) {
          @if (st.unrecognized.length + st.plugins.length; as n) {
            <section class="attention" aria-labelledby="home-attention" data-testid="home-attention">
              <h3 id="home-attention"><span i18n="@@home.view.attention">Needs attention</span> <span class="count faint">{{ n }}</span></h3>
              @for (u of st.unrecognized; track u.path) {
                <div class="callout">
                  <debrief-icon name="alert" [size]="16" class="warn" />
                  <div class="body"><debrief-path-text class="t" [value]="u.name" [full]="u.path" [copy]="true" /><div class="d">{{ u.reason }}</div></div>
                </div>
              }
              @for (p of st.plugins; track p.id) {
                <div class="callout">
                  <debrief-icon name="alert" [size]="16" class="warn" />
                  <div class="body"><div class="t wrap">{{ p.name }}</div><div class="d">{{ p.problem }}</div></div>
                </div>
              }
            </section>
          }
        }

        <section class="health" aria-labelledby="home-health">
          <h3 id="home-health" i18n="@@home.view.health">Health check</h3>
          <debrief-coverage-note [coverage]="home.health.coverage()" />
          <debrief-analysis-state [loading]="home.health.loading()" [error]="home.health.error()" [retryable]="home.health.retryable()" [hasData]="!!findings()" [empty]="false" (retry)="home.health.retry()">
            @if (findings(); as list) {
              @if (!list.length) {
                <div class="none" role="status">
                  <debrief-icon name="check" [size]="16" />
                  <div>
                    <strong i18n="@@home.view.none">Nothing alarming found</strong>
                    <p class="muted" i18n="@@home.view.noneDetail">No crashes, error spikes or unreadable files. Explore the timeline, or search the events for what you are after.</p>
                  </div>
                </div>
              } @else {
                <!-- Only the visible rows are rendered, however many findings the engine returns. -->
                <cdk-virtual-scroll-viewport class="findings" data-testid="home-findings" [itemSize]="rowPx()" [style.height.px]="listHeight()" role="list" aria-label="Findings" i18n-aria-label="@@home.view.findings">
                  <div *cdkVirtualFor="let f of shown(); trackBy: trackFinding" class="finding" role="listitem" [attr.data-severity]="f.severity" [style.height.px]="rowPx()">
                    <div class="text">
                      <div class="title" [title]="f.title"><span class="tag">{{ severityText(f) }}</span> {{ f.title }}</div>
                      <p class="detail" [title]="f.detail">{{ f.detail }}</p>
                    </div>
                    <div class="acts">
                      @for (b of buttons(f); track b.id) {
                        <button class="btn ghost link" type="button" (click)="act(f, b)">{{ b.label }}</button>
                      }
                    </div>
                  </div>
                </cdk-virtual-scroll-viewport>
                @if (list.length > FIRST_FINDINGS) {
                  <button class="btn ghost link all" type="button" data-testid="home-findings-toggle" (click)="showAll.set(!showAll())">
                    @if (showAll()) { <span i18n="@@home.view.showFewer">Show fewer</span> } @else { <span i18n="@@home.view.showAll">Show all {{ list.length }}</span> }
                  </button>
                }
              }
            }
          </debrief-analysis-state>
        </section>
        </div>

        <details class="feed" data-testid="home-feed-section">
          <summary id="home-feed" i18n="@@home.view.activity">Recent activity</summary>
          @if (feed().length) {
            <ul data-testid="home-feed">
              @for (item of feed(); track item.key) {
                <li [class.failed]="item.failed"><time class="faint">{{ clock(item.time) }}</time> {{ item.label }}@if (item.failed) {<span class="bad"> — <span i18n="@@home.view.failed">failed</span></span>}</li>
              }
            </ul>
          } @else {
            <p class="muted" i18n="@@home.view.noActivity">Nothing yet.</p>
          }
        </details>
      }

      <label class="auto" i18n="@@home.view.autoOpen">
        <input type="checkbox" [checked]="prefs.autoOpen()" (change)="prefs.setAutoOpen($any($event.target).checked)" />
        Show Home after opening files
      </label>
    </div>
  `,
  styles: `
    :host { container: home / inline-size; display: flex; flex-direction: column; height: 100%; min-height: 0; min-width: 0; background: var(--bg-1); color: var(--text); }
    .bar { flex: none; display: flex; flex-wrap: wrap; align-items: center; gap: 4px 8px; padding: 6px 12px; border-bottom: 1px solid var(--border); }
    .spacer { flex: 1; }
    .subtitle { min-width: 0; overflow-wrap: anywhere; }
    @container home (max-width: 479px) { .bar .subtitle { flex: 1 0 100%; order: -1; } .bar .spacer { display: none; } .bar { justify-content: flex-end; } }
    .scroll { flex: 1; min-height: 0; min-width: 0; overflow-x: hidden; overflow-y: auto; padding: 8px 12px 12px; display: flex; flex-direction: column; gap: 10px; }
    .scroll > * { flex: none; }
    .pair { display: grid; grid-template-columns: minmax(0, 1fr); gap: 10px 16px; align-items: start; }
    .pair > * { min-width: 0; }
    @container home (min-width: 640px) { .pair { grid-template-columns: minmax(280px, 1fr) minmax(0, 2fr); } }
    .feed summary { cursor: pointer; font-size: var(--fs-md); font-weight: 600; }
    .feed ul { margin-top: 6px; }
    .actions { display: flex; flex-wrap: wrap; gap: 6px; }
    .actions .btn { display: inline-flex; align-items: center; gap: 6px; }
    .pad { margin: 4px 0; }
    .subtitle { margin: 0; font-size: var(--fs-sm); color: var(--muted); font-variant-numeric: tabular-nums; }
    .tiles { margin: 0; display: flex; flex-wrap: wrap; gap: 8px; }
    .tiles .hero { flex: 1 1 calc(25% - 8px); min-width: min(160px, 100%); }
    @container home (max-width: 399px) { .tiles .hero { flex: 1 1 calc(50% - 8px); min-width: 0; } }
    .tiles > div { position: relative; }
    .tiles .hero { padding: 10px 12px; border: 1px solid var(--border-2); border-radius: var(--r-3); background: var(--bg-2); min-height: 60px; }
    .tiles .hero dt { font-size: var(--fs-sm); color: var(--muted); }
    .tiles .hero dd { margin: 2px 0 0; font-size: var(--fs-hero); line-height: 1.15; font-weight: 600; font-variant-numeric: tabular-nums; }
    .tiles .chip { flex: none; display: inline-flex; align-items: baseline; gap: 6px; padding: 2px 10px; border: 1px solid var(--border-2); border-radius: 999px; font-size: var(--fs-sm); width: max-content; max-width: 100%; }
    .tiles .chip dt { color: var(--muted); }
    .tiles .chip dd { margin: 0; font-weight: 600; font-variant-numeric: tabular-nums; }
    .tiles > div.bad dd { color: var(--danger-text); }
    .tiles > div.warn dd { color: var(--warn-text); }
    .cover { position: absolute; inset: 0; border: 0; border-radius: var(--r-3); background: transparent; cursor: pointer; }
    .cover:hover { background: var(--bg-hover); opacity: 0.35; }
    .cover:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 1px; }
    .spark { position: absolute; right: 10px; bottom: 10px; width: 40%; height: 20px; fill: none; stroke-width: 1.5; vector-effect: non-scaling-stroke; }
    .spark path { vector-effect: non-scaling-stroke; }
    .spark.bad { stroke: var(--danger-text); }
    .spark.warn { stroke: var(--warn-text); }
    .attention { display: flex; flex-direction: column; gap: 6px; }
    .callout { display: flex; gap: 10px; align-items: flex-start; padding: 5px 12px; border: 1px solid var(--border-2); border-inline-start: 3px solid var(--warn-text); border-radius: var(--r-2); background: var(--bg-2); }
    .callout .warn { color: var(--warn-text); margin-top: 1px; }
    .callout .body { flex: 1; min-width: 0; }
    .callout .t { min-width: 0; }
    .callout .t.wrap, .callout .d { overflow-wrap: anywhere; }
    .callout .t { font-size: var(--fs-md); font-weight: 600; }
    .callout .d { font-size: var(--fs-sm); color: var(--muted); }
    .count { font-weight: 400; font-variant-numeric: tabular-nums; }
    .link { color: var(--accent-text); }
    .all { align-self: flex-start; margin-top: 4px; }
    h3 { margin: 0 0 6px; font-size: var(--fs-md); font-weight: 600; display: flex; align-items: center; gap: 6px; }
    .chart { height: 150px; min-height: 130px; }
    .hint { font-size: var(--fs-xs); font-weight: 400; }
    .sub ul, .feed ul { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 3px; }
    .sub li, .feed li { font-size: var(--fs-sm); overflow-wrap: anywhere; }
    .feed li.failed { color: var(--danger-text); }
    .name { color: var(--text); }
    .muted { color: var(--muted); }
    .faint { color: var(--faint); }
    .bad { color: var(--danger-text); }
    .findings { border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); }
    .finding { box-sizing: border-box; /* width 0 + min-width 100%: the row follows the list's width instead of its longest title */ width: 0; min-width: 100%; height: 56px; display: flex; align-items: center; gap: 8px; padding: 6px 10px; border-bottom: 1px solid var(--border-2); }
    .finding .text { flex: 1; min-width: 0; }
    .detail, .title { overflow-wrap: anywhere; }
    .title { font-weight: 600; font-size: var(--fs-sm); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .tag { display: inline-block; padding: 0 5px; margin-right: 4px; border-radius: 4px; font-size: var(--fs-xs); font-weight: 600; border: 1px solid var(--border-3); color: var(--text-2); }
    [data-severity='critical'] .tag { border-color: var(--lvl-error); color: var(--danger-text); }
    [data-severity='warning'] .tag { border-color: var(--lvl-warn); color: var(--warn-text); }
    .detail { margin: 2px 0 0; font-size: var(--fs-sm); color: var(--text-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .acts { flex: none; display: flex; flex-wrap: wrap; gap: 6px; }
    :host(.narrow) .finding { flex-direction: column; align-items: stretch; gap: 4px; }
    :host(.narrow) .title, :host(.narrow) .detail { white-space: normal; display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; }
    :host(.narrow) .acts { flex: none; }
    .none { display: flex; gap: 10px; align-items: flex-start; padding: 12px; border: 1px solid var(--border-2); border-radius: 8px; background: var(--bg-2); }
    .none debrief-icon { color: var(--ok); margin-top: 2px; }
    .none p { margin: 4px 0 0; font-size: var(--fs-sm); }
    .auto { display: flex; align-items: center; gap: 8px; font-size: var(--fs-sm); color: var(--text-2); }
  `,
})
export class HomeView {
  protected readonly ai = inject(AiLauncher);
  protected readonly home = inject(HomeStore);
  protected readonly prefs = inject(HomePrefs);
  private readonly store = inject(WorkspaceStore);
  private readonly actions = inject(HealthActions);
  private readonly commands = inject(CommandRegistry);
  private readonly ctx = inject(COMMAND_CONTEXT);
  private readonly theme = inject(ChartTheme);
  private readonly recent = inject(RECENT_PROJECTS);

  protected readonly count = formatCount;
  protected readonly bytes = formatBytes;
  private readonly paneWidth = measuredWidth(0);
  /** The pane is too narrow for the actions beside the text. */
  protected readonly narrow = computed(() => (this.paneWidth() ?? NARROW_PANE_PX) < NARROW_PANE_PX);
  protected readonly rowPx = computed(() => (this.narrow() ? NARROW_ROW_PX : ROW_PX));
  protected readonly FIRST_FINDINGS = FIRST_FINDINGS;
  protected readonly showAll = signal(false);
  /** The findings on show: the first few, all on request. */
  protected readonly shown = computed(() => (this.showAll() ? (this.findings() ?? []) : (this.findings() ?? []).slice(0, FIRST_FINDINGS)));
  protected readonly viewBox = `0 0 ${SPARK_W} ${SPARK_H}`;

  protected readonly hasData = computed(() => (this.store.summary()?.sources.length ?? 0) + (this.store.summary()?.unrecognized.length ?? 0) > 0);
  protected readonly summary = this.home.summary;
  protected readonly findings = this.home.health.result;
  protected readonly lastProject = computed(() => this.recent.list()[0] ?? null);
  protected readonly crashes = computed(() => (this.findings() ?? []).filter((f) => f.kind === 'crash').length);
  protected readonly errorSpark = computed(() => sparklinePath(this.summary()?.errorTrend ?? []));
  protected readonly warningSpark = computed(() => sparklinePath(this.summary()?.warningTrend ?? []));
  protected readonly listHeight = computed(() => Math.min(this.shown().length, VISIBLE_ROWS) * this.rowPx());
  protected readonly stats = computed(() => {
    const summary = this.store.summary();
    return summary && this.hasData() ? healthStats(summary, this.store.instanceCount(), this.findings() ?? []) : null;
  });
  protected readonly span = computed(() => {
    const s = this.summary();
    if (!s || s.from === null || s.to === null) return $localize`:@@home.view.noTimestamps:No timestamps`;
    const tz = this.store.timezone();
    // Non-breaking spaces inside each value: the line can only break between the dates and the duration, never inside one.
    const unbreakable = (v: string): string => v.replace(/ /g, '\u00a0');
    const from = unbreakable(formatDateTime(s.from, tz));
    const to = unbreakable(formatDateTime(s.to, tz));
    const duration = unbreakable(formatDuration(s.to - s.from));
    return $localize`:@@home.view.span:${from}:from: → ${to}:to: (${duration}:duration:)`;
  });
  protected readonly option = computed(() => {
    const s = this.summary();
    if (!s) return null;
    const kinds = this.store.markerKindMap();
    const levels: Record<Level, string> = {
      Verbose: $localize`:@@home.view.level.verbose:Verbose`,
      Debug: $localize`:@@home.view.level.debug:Debug`,
      Information: $localize`:@@home.view.level.information:Information`,
      Warning: $localize`:@@home.view.level.warning:Warning`,
      Error: $localize`:@@home.view.level.error:Error`,
      Fatal: $localize`:@@home.view.level.fatal:Fatal`,
    };
    return historyOption(s, this.theme.tokens(), this.store.timezone(), {
      level: (l) => levels[l],
      marker: (k) => kinds.get(k)?.label ?? k,
      markerLane: $localize`:@@home.view.markerLane:Markers`,
      anomaly: $localize`:@@home.view.anomaly:Anomaly`,
    });
  });
  protected readonly feed = computed(() => feedItems(this.home.crumbs(), (c) => this.commands.find(c.operation)?.title ?? c.operation));

  constructor() {
    this.home.gate.use();
  }

  protected show(view: string): void {
    this.ctx.showView(view);
  }

  protected showLevels(levels: Level[]): void {
    this.ctx.setQuery({ levels });
    this.ctx.showView('events');
  }

  protected run(id: string): void {
    void this.commands.run(id, 'button');
  }

  protected clock(iso: string): string {
    return formatDateTime(Date.parse(iso), this.store.timezone()).split(' ').slice(2).join(' ');
  }

  protected selectBucket(ts: number): void {
    const s = this.summary();
    const range = s ? bucketRange(s, bucketAt(s, ts)) : null;
    if (range) this.ctx.setQuery({ from: range.from, to: range.to });
  }

  protected selectRange([from, to]: [number, number]): void {
    this.ctx.setQuery({ from, to });
  }

  protected trackFinding = (_: number, f: Finding): string => `${f.kind}:${f.title}:${f.detail}`;

  protected severityText(f: Finding): string {
    switch (f.severity) {
      case 'critical':
        return $localize`:@@home.view.tag.critical:critical`;
      case 'warning':
        return $localize`:@@home.view.tag.warning:warning`;
      default:
        return $localize`:@@home.view.tag.info:info`;
    }
  }

  protected buttons(f: Finding): ActionButton[] {
    return actionButtons(f.action);
  }

  protected act(f: Finding, b: ActionButton): void {
    void this.actions.run(f, b.id);
  }
}
