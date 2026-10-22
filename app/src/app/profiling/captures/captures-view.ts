import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { CaptureSummary } from '../../core/api.types';
import { formatCount, formatDateTime } from '../../core/format';
import { WorkspaceStore } from '../../core/workspace.store';
import { CommandRegistry } from '../../extensions/command-registry';
import { RecentService } from '../../core/recent/recent.service';
import { CapturesStore } from '../captures.store';
import { ProfileViewState } from '../profile-view-state';
import { captureKindLabel, formatBytes, formatNs, isHeap, isProfile } from '../profiling-format';
import { PROFILING_VIEW_IDS, ProfilingNavigator } from '../profiling-navigator';
import { PROFILING_STYLES } from '../profiling-styles';

/** The open captures: kind, source, duration, threads and import notes (truncations and limits). */
@Component({
  selector: 'imt-captures-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="bar">
      <span class="section-title" i18n="@@profiling.captures.title">Captures</span>
      <span class="spacer"></span>
      <button class="btn" type="button" [disabled]="store.loading()" (click)="open()" i18n="@@profiling.captures.open">Open performance capture…</button>
    </div>
    @if (store.error(); as e) {
      <div class="err" role="alert">
        <span class="msg">{{ e }}</span>
        <button class="btn" type="button" (click)="store.retry()" i18n="@@profiling.captures.retry">Retry</button>
        <button class="btn" type="button" (click)="store.dismiss()" i18n="@@profiling.captures.dismiss">Dismiss</button>
      </div>
    }
    @if (store.notice(); as n) { <div class="notice" role="status">{{ n }}</div> }
    <div class="body">
      @if (store.loading() && !store.captures().length) {
        <div class="state" role="status" i18n="@@profiling.captures.loading">Loading…</div>
      } @else if (!store.captures().length) {
        <div class="state" role="status" data-testid="captures-landing">
          <span i18n="@@profiling.captures.empty">No profiles or heap snapshots are open. Open a capture (speedscope, folded stacks, .cpuprofile, Chrome trace, pprof, perf script, .nettrace, .heapsnapshot), or drop one on the window.</span>
          <div class="landing">
            <button class="btn primary" type="button" [disabled]="store.loading()" (click)="open()" i18n="@@profiling.captures.landing.open">Open capture…</button>
            @for (a of landingActions(); track a.id) {
              <button class="btn" type="button" (click)="run(a.id)">{{ a.title }}</button>
            }
          </div>
          @if (recentCaptures().length) {
            <section class="recent" aria-labelledby="recent-captures-title" data-testid="recent-captures">
              <h3 id="recent-captures-title" class="section-title" i18n="@@profiling.captures.recent">Recent captures</h3>
              <ul>
                @for (c of recentCaptures(); track c.path) {
                  <li><button class="link" type="button" [title]="c.path" [disabled]="store.loading()" (click)="openRecent(c.path)">{{ fileName(c.path) }}</button></li>
                }
              </ul>
              <button class="mini" type="button" (click)="clearRecent()" i18n="@@profiling.captures.recent.clear">Clear recent captures</button>
            </section>
          }
          <p class="hint" i18n="@@profiling.captures.landing.hint">The hotspots, call tree and flame graph open once a capture is open.</p>
        </div>
      } @else {
        <ul class="list" aria-label="Open captures" i18n-aria-label="@@profiling.captures.listLabel">
          @for (c of store.captures(); track c.id) {
            <li class="cap" [class.active]="isActive(c)" [attr.aria-current]="isActive(c) ? 'true' : null">
              <div class="head">
                <button class="link name ellipsis" type="button" [title]="c.path ?? c.name" (click)="activate(c)">{{ c.name }}</button>
                <span class="tag">{{ kind(c) }}</span>
                @if (isActive(c)) { <span class="tag" i18n="@@profiling.captures.active">active</span> }
                <span class="spacer"></span>
                <button class="mini" type="button" [attr.aria-label]="closeLabel(c)" (click)="store.close(c.id)" i18n="@@profiling.captures.close">Close</button>
              </div>
              <dl class="facts">
                <dt i18n="@@profiling.captures.source">Source</dt><dd class="mono ellipsis" [title]="c.path ?? ''">{{ c.format }}{{ c.path ? ' · ' + c.path : '' }}</dd>
                @if (profile(c); as p) {
                  <dt i18n="@@profiling.captures.duration">Duration</dt><dd>{{ ns(p.durationNs) }}{{ p.startedMs !== null ? from(p.startedMs) : noStart }}</dd>
                  <dt i18n="@@profiling.captures.threads">Threads</dt><dd>{{ p.threads.length }}</dd>
                  <dt i18n="@@profiling.captures.samples">Samples</dt><dd i18n="@@profiling.captures.sampleCounts">{{ count(p.sampleCount) // i18n(ph: "samples") }} · {{ count(p.frameCount) // i18n(ph: "frames") }} frames · {{ count(p.eventCount) // i18n(ph: "events") }} events</dd>
                  <dt i18n="@@profiling.captures.metrics">Metrics</dt><dd>{{ metrics(p) }}</dd>
                }
                @if (heap(c); as h) {
                  <dt i18n="@@profiling.captures.taken">Taken</dt><dd>{{ h.takenMs !== null ? time(h.takenMs) : unknown }}</dd>
                  <dt i18n="@@profiling.captures.objects">Objects</dt><dd i18n="@@profiling.captures.objectCounts">{{ count(h.objectCount) // i18n(ph: "objects") }} · {{ count(h.typeCount) // i18n(ph: "types") }} types · {{ bytes(h.totalBytes) // i18n(ph: "bytes") }}</dd>
                  <dt i18n="@@profiling.captures.graph">Graph</dt><dd>{{ h.hasGraph ? graphYes : graphNo }}</dd>
                }
              </dl>
              @if (c.notes.length) {
                <div class="notes" role="note" [attr.aria-label]="notesLabel(c)">
                  <strong i18n="@@profiling.captures.notes">Import notes</strong>
                  <ul>@for (n of c.notes; track $index) { <li>{{ n }}</li> }</ul>
                </div>
              }
            </li>
          }
        </ul>
      }
    </div>
  `,
  styles: [
    PROFILING_STYLES,
    `
      .err { display: flex; align-items: center; gap: 8px; padding: 6px 12px; border-bottom: 1px solid var(--border); }
      .err .msg { color: var(--danger-text); white-space: pre-line; flex: 1; }
      .state { padding: 24px 12px; color: var(--muted); text-align: center; }
      .landing { display: flex; flex-wrap: wrap; justify-content: center; gap: 8px; margin-top: 12px; }
      .recent { margin-top: 16px; }
      .recent .link, .recent .mini { display: inline-flex; align-items: center; min-height: 24px; min-width: 24px; }
      .recent .mini { padding: 0 8px; }
      .recent ul { list-style: none; margin: 4px 0; padding: 0; }
      .recent li { margin: 2px 0; }
      .hint { font-size: var(--fs-xs); margin: 12px 0 0; }
      .cap { border: 1px solid var(--border-2); border-radius: 6px; padding: 8px 10px; margin-bottom: 8px; background: var(--bg-2); }
      .cap.active { border-color: var(--accent); }
      .head { display: flex; align-items: center; gap: 6px; }
      .name { font-weight: 600; color: var(--text); }
      .facts { display: grid; grid-template-columns: auto 1fr; gap: 2px 10px; margin: 6px 0 0; font-size: var(--fs-xs); }
      dt { color: var(--muted); }
      dd { margin: 0; min-width: 0; }
      .notes { margin-top: 6px; font-size: var(--fs-xs); color: var(--warn-text); }
      .notes ul { margin: 2px 0 0; padding-left: 16px; color: var(--text-2); }
    `,
  ],
})
export class CapturesView {
  protected readonly store = inject(CapturesStore);
  private readonly state = inject(ProfileViewState);
  private readonly nav = inject(ProfilingNavigator);
  private readonly workspace = inject(WorkspaceStore);
  private readonly commands = inject(CommandRegistry);
  private readonly recent = inject(RecentService);

  /** Captures opened before (the engine keeps the list): the landing offers them again. */
  protected readonly recentCaptures = this.recent.captures;

  protected fileName(path: string): string {
    return path.split(/[\\/]/).pop() || path;
  }

  protected async openRecent(path: string): Promise<void> {
    await this.show(await this.store.openPaths([path]));
  }

  protected clearRecent(): Promise<void> {
    return this.recent.clear('captures');
  }

  /** The commands that start a capture (profile a running process), when this build has them. */
  protected readonly landingActions = computed(() => {
    const ids = ['profiling.dotnet', 'profiling.native'];
    return this.commands.available().filter((c) => ids.includes(c.id));
  });

  protected run(id: string): void {
    void this.commands.run(id);
  }

  protected readonly count = formatCount;
  protected readonly ns = formatNs;
  protected readonly bytes = formatBytes;
  protected readonly kind = captureKindLabel;
  protected readonly profile = (c: CaptureSummary) => (isProfile(c) ? c : null);
  protected readonly heap = (c: CaptureSummary) => (isHeap(c) ? c : null);

  protected readonly noStart = $localize`:@@profiling.captures.noStart: (no wall-clock start)`;
  protected readonly unknown = $localize`:@@profiling.captures.unknown:unknown`;
  protected readonly graphYes = $localize`:@@profiling.captures.graphYes:yes: retained sizes, dominators and paths to roots`;
  protected readonly graphNo = $localize`:@@profiling.captures.graphNo:no: types only`;

  protected from(ms: number): string {
    const when = this.time(ms);
    return $localize`:@@profiling.captures.from: from ${when}:time:`;
  }

  protected closeLabel(c: CaptureSummary): string {
    return $localize`:@@profiling.captures.closeLabel:Close ${c.name}:name:`;
  }

  protected notesLabel(c: CaptureSummary): string {
    return $localize`:@@profiling.captures.notesLabel:Import notes for ${c.name}:name:`;
  }

  protected time(ms: number): string {
    return formatDateTime(ms, this.workspace.timezone(), true);
  }

  protected metrics(p: { metrics: { label: string; measurement: string }[] }): string {
    return p.metrics.map((m) => `${m.label} (${m.measurement})`).join(', ');
  }

  protected isActive(c: CaptureSummary): boolean {
    return c.capture === 'heap' ? this.state.heapId() === c.id : this.state.profileId() === c.id;
  }

  protected activate(c: CaptureSummary): void {
    if (isHeap(c)) this.nav.openHeap(c.id);
    else {
      this.state.setProfile(c.id);
      this.nav.show(PROFILING_VIEW_IDS.hotspots);
    }
  }

  protected async open(): Promise<void> {
    await this.show(await this.store.pickAndOpen());
  }

  private async show(opened: CaptureSummary[]): Promise<void> {
    if (opened.some(isProfile)) this.nav.show(PROFILING_VIEW_IDS.hotspots);
    else if (opened.some(isHeap)) this.nav.show(PROFILING_VIEW_IDS.heap);
  }
}
