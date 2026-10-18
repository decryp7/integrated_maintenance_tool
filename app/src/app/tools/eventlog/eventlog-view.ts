import { ChangeDetectionStrategy, Component, computed, effect, inject, InjectionToken, OnInit, signal } from '@angular/core';
import { formatCount, formatDateTime, formatNumber } from '../../core/format';
import { EVENT_LOG_BACKFILLS, EVENT_LOG_LEVELS, EventLogBackfill, EventLogChannelState } from '../../core/tools.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { LifecycleService } from '../../lifecycle/lifecycle.service';
import { Icon } from '../../shared/icon';
import { Switch } from '../../shared/switch';
import { HelpHint } from '../../ui/help-hint';
import { ToolError } from '../elevation-hint';
import { ToolUnavailable } from '../tool-unavailable';
import { BACKFILL_LABEL, LEVEL_LABEL } from './eventlog-model';
import { EventLogStore } from './eventlog.store';

/** How often the status is read while a run is going, in ms (tests shorten it). */
export const EVENT_LOG_POLL_MS = new InjectionToken<number>('EVENT_LOG_POLL_MS', { providedIn: 'root', factory: () => 1000 });

const STATE_LABEL: Record<EventLogChannelState, string> = {
  starting: $localize`:@@tools.eventlog.state.starting:Starting`,
  backfill: $localize`:@@tools.eventlog.state.backfill:Reading back`,
  live: $localize`:state of a followed channel|@@tools.eventlog.state.live:Live`,
  retrying: $localize`:@@tools.eventlog.state.retrying:Retrying`,
  stopped: $localize`:@@tools.eventlog.state.stopped:Stopped`,
};

/** Event Log follower: follows Windows event log channels into rolling CLEF files opened with live tail. */
@Component({
  selector: 'imt-eventlog-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, HelpHint, Switch, ToolError, ToolUnavailable],
  template: `
    <div class="scroll" role="region" aria-label="Event Log follower" i18n-aria-label="@@tools.eventlog.region" tabindex="0">
      <p class="intro" i18n="@@tools.eventlog.intro">Follow Windows event log channels as they are written. The events are saved as log files and opened here with live tail, so they show up in the grid, on the timeline and in Metrics.</p>
      <imt-tool-unavailable tool="event-log-follower" />
      @if (store.unsupported(); as reason) { <div class="warn" role="status" data-testid="eventlog-unsupported">{{ reason }}.</div> }
      <imt-tool-error [message]="store.error()" (dismissed)="store.dismissError()" />
      @if (store.notice(); as notice) { <div class="note" role="status">{{ notice }}</div> }

      <div class="card status" data-testid="eventlog-status" role="status">
        <span class="dot" [style.background]="store.running() ? (store.paused() ? 'var(--mk-warning)' : 'var(--mk-start)') : 'var(--mk-stop)'"></span>
        <strong>{{ runLabel() }}</strong>
        @if (store.dir(); as dir) { <span class="faint ellipsis" [title]="dir" i18n="@@tools.eventlog.output">Output: <span class="mono">{{ dir }}</span></span> }
        <span class="spacer"></span>
        <button type="button" class="icon-btn" aria-label="Refresh status" i18n-aria-label="@@tools.eventlog.refresh.aria" title="Refresh status" i18n-title="@@tools.eventlog.refresh.title" [disabled]="!store.canRun()" (click)="store.poll()"><imt-icon name="refresh" [size]="14" /></button>
      </div>

      @if (store.run(); as run) {
        <section class="card details" aria-labelledby="evl-progress" data-testid="eventlog-progress">
          <h2 id="evl-progress" class="section-title" i18n="@@tools.eventlog.progress">Progress</h2>
          <dl class="stats">
            <div><dt i18n="@@tools.eventlog.stat.events">Events</dt><dd data-testid="eventlog-events">{{ count(run.events) }}</dd></div>
            <div><dt i18n="@@tools.eventlog.stat.eventsPerSecond">Events/s</dt><dd>{{ rate(run.eventsPerSecond) }}</dd></div>
            <div><dt i18n="@@tools.eventlog.stat.queue">Queue</dt><dd>{{ count(run.queued) }} / {{ count(run.queueCapacity) }}</dd></div>
            <div><dt i18n="@@tools.eventlog.stat.unreadable">Unreadable</dt><dd>{{ count(run.unreadable) }}</dd></div>
          </dl>
          @if (run.lastEvent; as e) {
            <p class="last" data-testid="eventlog-last" i18n="@@tools.eventlog.lastEvent">Last event: <span class="mono">{{ time(e.ts) }}</span> · {{ e.channel }} · {{ e.provider }} · ID {{ e.eventId ?? '—' }} · {{ e.level }}</p>
          } @else {
            <p class="last faint" i18n="@@tools.eventlog.noEvents">No events written yet.</p>
          }
          <table class="channels">
            <caption class="sr-only" i18n="@@tools.eventlog.channelsCaption">Channels of this run</caption>
            <thead><tr><th scope="col" i18n="@@tools.eventlog.col.channel">Channel</th><th scope="col" i18n="@@tools.eventlog.col.state">State</th><th scope="col" class="num" i18n="@@tools.eventlog.col.events">Events</th><th scope="col" class="num" i18n="@@tools.eventlog.col.lastRecord">Last record</th></tr></thead>
            <tbody>
              @for (c of run.channels; track c.channel) {
                <tr>
                  <td>{{ c.channel }}</td>
                  <td><span class="state" [class]="'state s-' + c.state">{{ stateLabel[c.state] }}</span>
                    @if (c.error; as err) { <div class="problem">{{ err }}</div> }</td>
                  <td class="num">{{ count(c.events) }}</td>
                  <td class="num mono">{{ c.lastRecordId ?? '—' }}</td>
                </tr>
              }
            </tbody>
          </table>
          @if (run.errors.length) {
            <h3 class="sub" i18n="@@tools.eventlog.recentErrors">Recent errors</h3>
            <ul class="errors" data-testid="eventlog-errors">
              @for (e of run.errors; track $index) {
                <li><span class="mono faint">{{ time(e.at) }}</span> @if (e.channel) { <strong>{{ e.channel }}</strong> } {{ e.message }}</li>
              }
            </ul>
          }
          <p class="faint small" i18n="@@tools.eventlog.liveQuery.p">Live query: <span class="mono">{{ run.xpath }}</span></p>
        </section>
      }

      <fieldset class="form" [disabled]="store.running()" (input)="edited.set(true)" (change)="edited.set(true)">
        <section aria-labelledby="evl-channels">
          <h2 id="evl-channels" class="section-title" i18n="@@tools.eventlog.channels">Channels <imt-help label="Channels" i18n-label="@@tools.eventlog.channels.helpLabel" text="The classic channels and the Microsoft-Windows-*/Operational channels of this machine. Reading Security needs administrator rights." i18n-text="@@tools.eventlog.channels.helpText" /></h2>
          <div class="inline">
            <input class="input grow" type="search" aria-label="Search channels" i18n-aria-label="@@tools.eventlog.search.aria" placeholder="Search channels" i18n-placeholder="@@tools.eventlog.search.placeholder" [value]="store.search()" (input)="store.setSearch(val($event))" />
            <span class="faint" data-testid="eventlog-selected" i18n="@@tools.eventlog.selected">{{ store.draft().channels.length }} selected</span>
            @if (store.draft().channels.length) { <button type="button" class="btn ghost" (click)="store.clearChannels(); edited.set(true)" i18n="clear the channel selection|@@tools.eventlog.clearChannels">Clear</button> }
          </div>
          <div class="picker" role="group" aria-label="Channels to follow" i18n-aria-label="@@tools.eventlog.picker.aria">
            @for (c of store.visibleChannels(); track c.name) {
              <label class="check">
                <input type="checkbox" [checked]="store.draft().channels.includes(c.name)" (change)="store.toggleChannel(c.name, checked($event))" />
                <span class="ellipsis">{{ c.name }}</span>
                @if (c.needsAdmin) { <span class="badge" title="Needs administrator rights" i18n-title="@@tools.eventlog.admin.title" i18n="@@tools.eventlog.admin.badge">Admin</span> }
              </label>
            } @empty {
              @if (store.channels().length) { <span class="faint" i18n="@@tools.eventlog.noMatch">No channel matches the search.</span> } @else { <span class="faint" i18n="@@tools.eventlog.noChannels">No channels loaded.</span> }
            }
          </div>
          @if (store.needsAdmin().length) {
            <div class="warn" role="status" data-testid="eventlog-admin-note">
              <ng-container i18n="@@tools.eventlog.needsAdmin">{{ store.needsAdmin().join(', ') }} needs administrator rights.</ng-container>
              @if (life.canRestartElevated()) {
                <button type="button" class="btn" (click)="life.restartElevated()" i18n="@@tools.eventlog.restartAdmin">Restart as administrator</button>
              }
            </div>
          }
        </section>

        <section aria-labelledby="evl-filter">
          <h2 id="evl-filter" class="section-title" i18n="noun, section heading|@@tools.eventlog.filter">Filter</h2>
          <div class="inline" role="group" aria-label="Levels" i18n-aria-label="@@tools.eventlog.levels.aria">
            @for (l of levels; track l) {
              <label class="check"><input type="checkbox" [checked]="store.draft().levels.includes(l)" (change)="store.toggleLevel(l, checked($event))" /> {{ levelLabel[l] }}</label>
            }
          </div>
          <label class="lbl"><ng-container i18n="@@tools.eventlog.xpath">XPath filter (optional)</ng-container> <imt-help label="XPath filter" i18n-label="@@tools.eventlog.xpath.helpLabel" text="An Event Log XPath such as *[System[EventID=7036]]. It is combined with the levels and the backfill window; Validate shows the final query." i18n-text="@@tools.eventlog.xpath.helpText" />
            <textarea class="input mono" rows="2" [value]="store.draft().xpath" placeholder="*[System[Provider[@Name='Service Control Manager']]]" (input)="store.patchDraft({ xpath: val($event) })"></textarea>
          </label>
          <div class="inline">
            <button type="button" class="btn" [disabled]="!!store.busy() || !store.canRun() || store.problems().length > 0" (click)="store.validate()" i18n="@@tools.eventlog.validate">Validate</button>
            <span class="faint small" i18n="@@tools.eventlog.validate.hint">Checks every chosen channel with the final query, without starting.</span>
          </div>
          @if (store.validation(); as v) {
            <div class="card validation" data-testid="eventlog-validation">
              <div i18n="@@tools.eventlog.liveQuery.div">Live query: <span class="mono">{{ v.xpath }}</span></div>
              @if (v.backfillXpath; as b) { <div i18n="@@tools.eventlog.backfillQuery">Backfill query: <span class="mono">{{ b }}</span></div> }
              <ul class="checks">
                @for (c of v.channels; track c.name) {
                  <li [class.bad]="!c.ok"><imt-icon [name]="c.ok ? 'check' : 'alert'" [size]="12" /> <strong>{{ c.name }}</strong> {{ c.ok ? okText : c.error }}</li>
                }
              </ul>
            </div>
          }
        </section>

        <section aria-labelledby="evl-run">
          <h2 id="evl-run" class="section-title" i18n="noun, section heading|@@tools.eventlog.run">Run</h2>
          <div class="row">
            <label class="lbl"><ng-container i18n="@@tools.eventlog.readBack">Read back first</ng-container>
              <select class="select" [value]="store.draft().backfill" (change)="store.patchDraft({ backfill: backfill($event) })">
                @for (b of backfills; track b) { <option [value]="b">{{ backfillLabel[b] }}</option> }
              </select>
            </label>
            <label class="lbl"><ng-container i18n="@@tools.eventlog.outputFolder">Output folder (optional, defaults to IMT's data folder)</ng-container>
              <input class="input mono" [value]="store.draft().dir" (input)="store.patchDraft({ dir: val($event) })" />
            </label>
          </div>
          <imt-switch [checked]="store.draft().resume" (checkedChange)="store.patchDraft({ resume: $event }); edited.set(true)" i18n="@@tools.eventlog.resume">Continue from the last position</imt-switch>
        </section>
      </fieldset>

      @if (edited()) {
        @for (p of store.problems(); track p) { <div class="problem">{{ p }}</div> }
      }
      <div class="inline">
        @if (store.running()) {
          @if (store.paused()) {
            <button type="button" class="btn" [disabled]="!!store.busy()" (click)="store.resume()" i18n="@@tools.eventlog.btn.resume">Resume</button>
          } @else {
            <button type="button" class="btn" [disabled]="!!store.busy()" (click)="store.pause()" i18n="@@tools.eventlog.btn.pause">Pause</button>
          }
          <button type="button" class="btn primary" [disabled]="!!store.busy()" (click)="store.stop()" i18n="@@tools.eventlog.btn.stop">Stop</button>
        } @else {
          <button type="button" class="btn primary" [disabled]="!!store.busy() || store.problems().length > 0 || !store.canRun()" (click)="store.start()" i18n="@@tools.eventlog.btn.start">Start</button>
        }
        @if (store.busy(); as busy) { <span class="faint" role="status">{{ busy }}</span> }
      </div>
    </div>
  `,
  styles: `
    :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .scroll { flex: 1; min-height: 0; overflow: auto; padding: 14px 18px; display: flex; flex-direction: column; gap: 12px; max-width: 820px; }
    .scroll:focus-visible { outline-offset: -2px; }
    section, .form { display: flex; flex-direction: column; gap: 8px; }
    .form { border: 0; margin: 0; padding: 0; min-width: 0; gap: 14px; }
    h2 { margin: 0; display: flex; align-items: center; gap: 6px; }
    .sub { margin: 4px 0 0; font-size: 12px; color: var(--muted); }
    .intro { margin: 0; color: var(--text-2); line-height: 1.5; }
    .card { padding: 8px 12px; border: 1px solid var(--border-3); border-radius: 8px; background: var(--bg-2); }
    .status { display: flex; align-items: center; gap: 10px; }
    .dot { width: 8px; height: 8px; border-radius: 50%; flex: none; }
    .spacer { flex: 1; }
    .inline { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
    .grow { flex: 1; min-width: 160px; }
    .row { display: flex; gap: 12px; flex-wrap: wrap; }
    .row .lbl { flex: 1; min-width: 180px; }
    .lbl { display: flex; flex-direction: column; gap: 4px; font-size: 11px; color: var(--muted); }
    .check { display: inline-flex; gap: 6px; align-items: center; color: var(--text-2); min-width: 0; }
    .picker { display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: 4px 12px; max-height: 220px; overflow: auto; padding: 6px 8px; border: 1px solid var(--border-3); border-radius: 6px; background: var(--bg-2); }
    .badge { flex: none; font-size: 10px; padding: 0 5px; border-radius: 8px; border: 1px solid var(--chip-border); background: var(--chip-bg); color: var(--chip-text); }
    .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .problem { color: var(--danger-text); font-size: 12px; }
    .note { padding: 6px 10px; border-radius: 6px; border: 1px solid var(--ok); background: var(--bg-3); font-size: 12px; }
    .warn { padding: 8px 10px; border-radius: 6px; border: 1px solid var(--lvl-warn); background: var(--bg-3); font-size: 12px; display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
    .details { display: flex; flex-direction: column; gap: 8px; }
    .stats { display: flex; gap: 24px; flex-wrap: wrap; margin: 0; }
    .stats dt { font-size: 11px; color: var(--muted); }
    .stats dd { margin: 0; font-size: 16px; font-variant-numeric: tabular-nums; }
    .last { margin: 0; font-size: 12px; overflow-wrap: anywhere; }
    .small { font-size: 11px; margin: 0; overflow-wrap: anywhere; }
    .channels { width: 100%; border-collapse: collapse; font-size: 12px; }
    .channels th { text-align: left; font-weight: 500; color: var(--muted); border-bottom: 1px solid var(--border-3); padding: 3px 6px; }
    .channels td { padding: 3px 6px; border-bottom: 1px solid var(--border); vertical-align: top; overflow-wrap: anywhere; }
    .num { text-align: right; font-variant-numeric: tabular-nums; }
    .state::before { content: ''; display: inline-block; width: 7px; height: 7px; border-radius: 50%; margin-right: 5px; background: var(--mk-stop); }
    .s-live::before { background: var(--mk-start); }
    .s-backfill::before, .s-starting::before { background: var(--accent); }
    .s-retrying::before { background: var(--mk-warning); }
    .errors, .checks { margin: 0; padding-left: 0; list-style: none; display: flex; flex-direction: column; gap: 3px; font-size: 12px; }
    .checks li { display: flex; gap: 6px; align-items: baseline; overflow-wrap: anywhere; }
    .checks li.bad { color: var(--danger-text); }
    .validation { display: flex; flex-direction: column; gap: 6px; font-size: 12px; overflow-wrap: anywhere; }
    .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
  `,
})
export class EventLogView implements OnInit {
  /** Validation messages wait until the user has changed the form (Start stays disabled meanwhile). */
  protected readonly edited = signal(false);
  protected readonly store = inject(EventLogStore);
  protected readonly life = inject(LifecycleService);
  private readonly workspace = inject(WorkspaceStore);
  protected readonly levels = EVENT_LOG_LEVELS;
  protected readonly levelLabel = LEVEL_LABEL;
  protected readonly backfills = EVENT_LOG_BACKFILLS;
  protected readonly backfillLabel = BACKFILL_LABEL;
  protected readonly stateLabel = STATE_LABEL;
  protected readonly okText = $localize`:@@tools.eventlog.ok:OK`;
  protected readonly runLabel = computed(() => {
    if (!this.store.running()) return $localize`:@@tools.eventlog.run.stopped:Stopped`;
    return this.store.paused() ? $localize`:@@tools.eventlog.run.paused:Paused` : $localize`:@@tools.eventlog.run.running:Running`;
  });

  constructor() {
    const every = inject(EVENT_LOG_POLL_MS);
    // Reads the status while a run is going; stops with the run and when the panel closes.
    effect((onCleanup) => {
      if (!this.store.running()) return;
      const timer = setInterval(() => void this.store.poll(), every);
      onCleanup(() => clearInterval(timer));
    });
  }

  ngOnInit(): void {
    void this.store.load();
  }

  protected val = (e: Event) => (e.target as HTMLInputElement).value;
  protected checked = (e: Event) => (e.target as HTMLInputElement).checked;
  protected backfill = (e: Event) => this.val(e) as EventLogBackfill;
  protected count = formatCount;
  protected rate = (v: number) => formatNumber(v);
  protected time = (ts: number | null) => formatDateTime(ts, this.workspace.timezone(), true);
}
