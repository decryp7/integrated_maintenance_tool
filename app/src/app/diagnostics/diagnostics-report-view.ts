import { ChangeDetectionStrategy, Component, computed, inject, OnInit } from '@angular/core';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { activeLocale } from '../i18n/locale';
import { ChildRunState, ProcessSummary, ReportSnapshot } from './diagnostics-report.types';
import { DiagnosticsReportStore } from './diagnostics-report.store';
import { SnapshotBreadcrumb } from '../core/api.types';

/** How far around a breadcrumb the timeline zooms (each side). */
export const BREADCRUMB_WINDOW_MS = 30_000;

/** Keys of the system info shown as a list, in order; the plugins have their own table. */
const SYSTEM_KEYS = ['appVersion', 'commit', 'os', 'osVersion', 'arch', 'cpuCores', 'totalMemoryMb', 'availableMemoryMb', 'safeMode'] as const;

/** Read-only view of an opened IMT diagnostics zip: system, child processes, steps, breadcrumbs and error snapshots. */
@Component({
  selector: 'imt-diagnostics-report-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (store.error(); as e) {
      <div class="msg err" role="alert" data-testid="diag-error">
        <span>{{ e }}</span>
        <button type="button" class="btn" (click)="retry()" i18n="@@diagnostics.reportView.retry">Retry</button>
      </div>
    }
    @if (store.report(); as r) {
      <div class="wrap">
        <header class="head" data-testid="diag-header">
          <h2>{{ r.name }}</h2>
          <p class="sub" i18n="@@diagnostics.reportView.subtitle">IMT {{ r.manifest.imtVersion }} ({{ r.manifest.commit }}), created {{ when(r.manifest.createdAt) }}, {{ r.manifest.files.length }} files</p>
          @if (r.unreadable.length) {
            <div class="msg warn" role="status" data-testid="diag-unreadable">
              <span i18n="@@diagnostics.reportView.unreadable">Some files of the report could not be read:</span>
              <span class="mono">{{ r.unreadable.join(', ') }}</span>
            </div>
          }
          @if (r.description) {
            <blockquote class="description" data-testid="diag-description">{{ r.description }}</blockquote>
          }
        </header>

        <section aria-labelledby="diag-system-title">
          <h3 id="diag-system-title" i18n="@@diagnostics.reportView.system">System</h3>
          <dl data-testid="diag-system">
            @for (row of systemRows(); track row.key) {
              <dt>{{ row.label }}</dt>
              <dd>{{ row.value }}</dd>
            }
          </dl>
          @if (plugins().length) {
            <table aria-label="Plugins" i18n-aria-label="@@diagnostics.reportView.pluginsTable" data-testid="diag-plugins">
              <thead><tr><th scope="col" i18n="@@diagnostics.reportView.pluginId">Plugin</th><th scope="col" i18n="@@diagnostics.reportView.pluginVersion">Version</th><th scope="col" i18n="@@diagnostics.reportView.pluginStatus">Status</th></tr></thead>
              <tbody>
                @for (p of plugins(); track p.id) {
                  <tr><td class="mono">{{ p.id }}</td><td>{{ p.version }}</td><td>{{ p.status }}</td></tr>
                }
              </tbody>
            </table>
          }
        </section>

        <section aria-labelledby="diag-processes-title">
          <h3 id="diag-processes-title" i18n="@@diagnostics.reportView.processes">Processes</h3>
          @if (r.processes.length) {
            <table data-testid="diag-processes" aria-labelledby="diag-processes-title">
              <thead>
                <tr>
                  <th scope="col" i18n="@@diagnostics.reportView.colName">Name</th>
                  <th scope="col" i18n="@@diagnostics.reportView.colKind">Kind</th>
                  <th scope="col" i18n="@@diagnostics.reportView.colState">State</th>
                  <th scope="col" i18n="@@diagnostics.reportView.colPid">PID</th>
                  <th scope="col" i18n="@@diagnostics.reportView.colRestarts">Restarts</th>
                  <th scope="col" i18n="@@diagnostics.reportView.colLastExit">Last exit</th>
                  <th scope="col" i18n="@@diagnostics.reportView.colQuarantine">Quarantine reason</th>
                </tr>
              </thead>
              <tbody>
                @for (p of r.processes; track p.name) {
                  <tr [attr.data-state]="p.state">
                    <td class="mono">{{ p.name }}</td>
                    <td>{{ p.kind }}</td>
                    <td><span class="state" [class]="'state ' + p.state">{{ stateLabel(p.state) }}</span></td>
                    <td>{{ p.pid ?? '' }}</td>
                    <td>{{ p.restarts }}</td>
                    <td>{{ lastExit(p) }}</td>
                    <td>{{ p.quarantineReason ?? '' }}</td>
                  </tr>
                }
              </tbody>
            </table>
          } @else {
            <p class="muted" i18n="@@diagnostics.reportView.noProcesses">No child processes were recorded.</p>
          }
        </section>

        <section aria-labelledby="diag-steps-title">
          <h3 id="diag-steps-title" i18n="@@diagnostics.reportView.steps">Steps to reproduce</h3>
          @if (r.steps.length) {
            <ol class="steps" data-testid="diag-steps" aria-labelledby="diag-steps-title">
              @for (step of r.steps; track $index) {
                <li>{{ stripNumber(step) }}</li>
              }
            </ol>
          } @else {
            <p class="muted" data-testid="diag-steps" i18n="@@diagnostics.reportView.noSteps">No activity was recorded.</p>
          }
        </section>

        <section aria-labelledby="diag-crumbs-title">
          <h3 id="diag-crumbs-title" i18n="@@diagnostics.reportView.breadcrumbs">Breadcrumbs</h3>
          @if (r.breadcrumbs.length) {
            <ul class="crumbs" data-testid="diag-breadcrumbs" aria-labelledby="diag-crumbs-title">
              @for (b of r.breadcrumbs; track $index) {
                <li>
                  <button type="button" class="crumb" [title]="zoomHint" (click)="zoomTo(b)">
                    <span class="time">{{ when(b.time) }}</span>
                    <span class="op mono">{{ b.operation }}</span>
                    <span class="trigger">{{ b.trigger }}</span>
                    @if (b.outcome; as o) { <span class="outcome" [class.bad]="o === 'error'">{{ o }}</span> }
                  </button>
                </li>
              }
            </ul>
          } @else {
            <p class="muted" i18n="@@diagnostics.reportView.noBreadcrumbs">No breadcrumbs were recorded.</p>
          }
        </section>

        <section aria-labelledby="diag-snaps-title">
          <h3 id="diag-snaps-title" i18n="@@diagnostics.reportView.snapshots">Error snapshots</h3>
          @if (r.snapshots.length) {
            <ul class="grid" aria-labelledby="diag-snaps-title">
              @for (s of r.snapshots; track s.id) {
                <li class="card" [class.open]="store.selected() === s.id" data-testid="diag-snapshot-card">
                  <button type="button" class="thumb" [attr.aria-label]="openLabel(label(s))" (click)="store.show(s.id)">
                    @if (store.images()[s.id]; as src) {
                      <img [src]="src" alt="" loading="lazy" />
                    } @else {
                      <span class="ph" i18n="@@diagnostics.reportView.noPreview">No preview</span>
                    }
                  </button>
                  <div class="meta">
                    <span class="when">{{ when(s.createdAt) }}</span>
                    <span class="reason">{{ s.reason }}</span>
                    <span class="text imt-selectable" [title]="s.message">{{ s.message }}</span>
                  </div>
                </li>
              }
            </ul>
          } @else {
            <p class="muted" i18n="@@diagnostics.reportView.noSnapshots">The report has no error snapshots.</p>
          }
          @if (store.current(); as c) {
            <section class="detail imt-selectable" aria-label="Snapshot details" i18n-aria-label="@@diagnostics.reportView.details" data-testid="diag-snapshot-detail">
              <div class="dhead">
                <strong>{{ when(c.createdAt) }}</strong>
                <span class="grow"></span>
                <button type="button" class="btn ghost" (click)="store.show(null)" i18n="Close the details|Button@@diagnostics.reportView.close">Close</button>
              </div>
              @if (store.images()[c.id]; as src) {
                <img class="big" [src]="src" alt="Screenshot taken when the error happened" i18n-alt="@@diagnostics.reportView.screenshotAlt" data-testid="diag-snapshot-image" />
              } @else if (store.imageErrors()[c.id]; as err) {
                <div class="msg err" role="alert">{{ err }}</div>
              }
              <dl>
                <dt i18n="@@diagnostics.reportView.reason">Reason</dt><dd>{{ c.reason }}</dd>
                <dt i18n="@@diagnostics.reportView.message">Message</dt><dd class="mono">{{ c.message }}</dd>
                @if (c.view) { <dt i18n="@@diagnostics.reportView.activeView">Active view</dt><dd>{{ c.view }}</dd> }
                <dt i18n="@@diagnostics.reportView.id">Id</dt><dd class="mono">{{ c.id }}</dd>
              </dl>
              @if (c.steps.length) {
                <h4 id="diag-snapshot-steps-title" i18n="@@diagnostics.reportView.snapshotSteps">Steps to reproduce</h4>
                <ol class="steps" aria-labelledby="diag-snapshot-steps-title" data-testid="diag-snapshot-steps">
                  @for (step of c.steps; track $index) {
                    <li>{{ stripNumber(step) }}</li>
                  }
                </ol>
              }
            </section>
          }
        </section>
      </div>
    } @else if (!store.error()) {
      <p class="empty" data-testid="diag-empty">
        @if (store.loading()) {
          <ng-container i18n="@@diagnostics.reportView.loading">Loading…</ng-container>
        } @else {
          <ng-container i18n="@@diagnostics.reportView.empty">No diagnostics report is open. Open a zip saved with Help ▸ Report a problem… (File ▸ Open, or drop it here) to see its processes, steps and snapshots.</ng-container>
        }
      </p>
    }
  `,
  styles: `
    :host { flex: 1; min-width: 0; min-height: 0; display: block; overflow: auto; background: var(--bg-1); color: var(--text); font-size: var(--fs-sm); }
    .wrap { display: flex; flex-direction: column; gap: 14px; padding: 12px 16px; }
    h2 { margin: 0; font-size: var(--fs-lg); overflow-wrap: anywhere; }
    h3 { margin: 0 0 6px; font-size: var(--fs-md); }
    h4 { margin: 4px 0 0; font-size: var(--fs-sm); }
    .sub, .muted { margin: 2px 0 0; color: var(--text-2); }
    .empty { margin: 0; padding: 16px; color: var(--text-2); }
    .description { margin: 6px 0 0; padding: 4px 10px; border-inline-start: 3px solid var(--border-2); color: var(--text-2); white-space: pre-wrap; overflow-wrap: anywhere; }
    .msg { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin: 6px 0 0; padding: 6px 10px; border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-3); overflow-wrap: anywhere; }
    .msg.err { border-color: var(--lvl-error); color: var(--danger-text); }
    .msg.warn { border-color: var(--lvl-warn); color: var(--warn-text); }
    :host > .msg { margin: 10px 16px; }
    dl { margin: 0 0 8px; display: grid; grid-template-columns: max-content 1fr; gap: 4px 12px; }
    dt { color: var(--text-2); }
    dd { margin: 0; overflow-wrap: anywhere; }
    table { border-collapse: collapse; width: 100%; }
    th, td { text-align: start; padding: 4px 8px; border-bottom: 1px solid var(--border-2); }
    th { color: var(--text-2); font-weight: 600; }
    .state { font-weight: 600; }
    .state.crashed { color: var(--danger-text); }
    .state.quarantined { color: var(--warn-text); }
    .state.running { color: var(--ok); }
    tr[data-state=crashed] td { background: color-mix(in srgb, var(--lvl-error) 10%, transparent); }
    tr[data-state=quarantined] td { background: color-mix(in srgb, var(--lvl-warn) 10%, transparent); }
    .steps { margin: 0; padding-inline-start: 22px; display: flex; flex-direction: column; gap: 2px; }
    .crumbs { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
    .crumb { all: unset; box-sizing: border-box; width: 100%; display: flex; gap: 12px; align-items: baseline; padding: 3px 8px; border-radius: 4px; cursor: pointer; }
    .crumb:hover { background: var(--bg-3); }
    .crumb:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
    .time { color: var(--text-2); min-width: 150px; }
    .trigger { color: var(--text-2); }
    .outcome.bad { color: var(--danger-text); }
    .grid { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(auto-fill, minmax(190px, 1fr)); gap: 10px; }
    .card { border: 1px solid var(--border-2); border-radius: 8px; background: var(--bg-2); display: flex; flex-direction: column; overflow: hidden; }
    .card.open { border-color: var(--accent); }
    .thumb { all: unset; display: block; cursor: pointer; height: 110px; background: var(--bg-3); }
    .thumb:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
    .thumb img { width: 100%; height: 100%; object-fit: cover; object-position: top left; display: block; image-rendering: pixelated; }
    .ph { display: grid; place-items: center; height: 100%; color: var(--muted); }
    .meta { display: flex; flex-direction: column; gap: 2px; padding: 6px 8px; min-width: 0; }
    .reason { color: var(--warn-text); }
    .text { color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .detail { margin-top: 10px; border: 1px solid var(--border-2); border-radius: 8px; background: var(--bg-2); padding: 10px; display: flex; flex-direction: column; gap: 8px; }
    .dhead { display: flex; gap: 8px; align-items: center; }
    .grow { flex: 1; }
    .big { max-width: 100%; height: auto; border: 1px solid var(--border-2); border-radius: 4px; }
  `,
})
export class DiagnosticsReportPanel implements OnInit {
  protected readonly store = inject(DiagnosticsReportStore);
  private readonly ctx = inject(COMMAND_CONTEXT);
  protected readonly zoomHint = $localize`:@@diagnostics.reportView.zoomHint:Zoom the timeline to this moment`;

  protected readonly systemRows = computed(() => {
    const info = this.store.report()?.systemInfo ?? {};
    const labels: Record<(typeof SYSTEM_KEYS)[number], string> = {
      appVersion: $localize`:@@diagnostics.reportView.sys.appVersion:IMT version`,
      commit: $localize`:@@diagnostics.reportView.sys.commit:Build`,
      os: $localize`:@@diagnostics.reportView.sys.os:Operating system`,
      osVersion: $localize`:@@diagnostics.reportView.sys.osVersion:OS version`,
      arch: $localize`:@@diagnostics.reportView.sys.arch:Architecture`,
      cpuCores: $localize`:@@diagnostics.reportView.sys.cpuCores:CPU cores`,
      totalMemoryMb: $localize`:@@diagnostics.reportView.sys.totalMemoryMb:Memory (MB)`,
      availableMemoryMb: $localize`:@@diagnostics.reportView.sys.availableMemoryMb:Available memory (MB)`,
      safeMode: $localize`:@@diagnostics.reportView.sys.safeMode:Safe mode`,
    };
    return SYSTEM_KEYS.filter((k) => info[k] !== undefined && info[k] !== null).map((k) => ({ key: k, label: labels[k], value: this.show(info[k]) }));
  });

  /** The plugin list of the system info, typed at the edge (it is untrusted JSON). */
  protected readonly plugins = computed(() => {
    const raw = this.store.report()?.systemInfo['plugins'];
    if (!Array.isArray(raw)) return [];
    return raw
      .filter((p): p is Record<string, unknown> => typeof p === 'object' && p !== null)
      .map((p) => ({ id: this.show(p['id']), version: this.show(p['version']), status: this.show(p['status']) }));
  });

  ngOnInit(): void {
    if (!this.store.report()) void this.retry();
  }

  protected async retry(): Promise<void> {
    await this.store.refresh();
    void this.store.loadImages();
  }

  /** Zooms the timeline to a window around the moment of a breadcrumb. */
  protected zoomTo(b: SnapshotBreadcrumb): void {
    const at = Date.parse(b.time);
    if (Number.isNaN(at)) return;
    this.ctx.setQuery({ from: at - BREADCRUMB_WINDOW_MS, to: at + BREADCRUMB_WINDOW_MS });
  }

  protected stateLabel(state: ChildRunState): string {
    switch (state) {
      case 'running': return $localize`:@@diagnostics.reportView.state.running:running`;
      case 'exited': return $localize`:@@diagnostics.reportView.state.exited:exited`;
      case 'stopped': return $localize`:@@diagnostics.reportView.state.stopped:stopped`;
      case 'crashed': return $localize`:@@diagnostics.reportView.state.crashed:crashed`;
      case 'quarantined': return $localize`:@@diagnostics.reportView.state.quarantined:quarantined`;
      case 'unknown': return $localize`:@@diagnostics.reportView.state.unknown:unknown`;
    }
  }

  protected lastExit(p: ProcessSummary): string {
    if (!p.lastExit) return '';
    return p.lastExitAt ? `${p.lastExit} (${this.when(p.lastExitAt)})` : p.lastExit;
  }

  protected openLabel(snapshot: string): string {
    return $localize`:@@diagnostics.reportView.openOne:Open snapshot ${snapshot}:snapshot:`;
  }

  protected label(s: ReportSnapshot): string {
    return `${this.when(s.createdAt)} ${s.reason}`;
  }

  protected when(iso: string): string {
    const t = new Date(iso);
    return Number.isNaN(t.getTime()) ? iso : t.toLocaleString(activeLocale());
  }

  /** The engine numbers its steps; the list numbers them again. */
  protected stripNumber(step: string): string {
    return step.replace(/^\d+\.\s*/, '');
  }

  private show(v: unknown): string {
    if (typeof v === 'boolean') return v ? $localize`:@@diagnostics.reportView.yes:yes` : $localize`:@@diagnostics.reportView.no:no`;
    if (typeof v === 'number') return String(v);
    return typeof v === 'string' ? v : '';
  }
}
