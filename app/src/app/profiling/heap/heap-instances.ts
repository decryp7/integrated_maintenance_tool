import { formatExactBytes } from '../../shared/format/units';
import { ChangeDetectionStrategy, Component, effect, inject, input, untracked } from '@angular/core';
import { HeapObjectsPage, HeapPaths } from '../../core/api.types';
import { formatCount } from '../../core/format';
import { AnalysisState } from '../../analysis/analysis-state';
import { AsyncResource } from '../../analysis/async-resource';
import { ProfilingBackend } from '../profiling-backend';
import { formatBytes } from '../profiling-format';
import { PROFILING_STYLES } from '../profiling-styles';
import { HeapSelection } from './heap-selection';

/** Instances of a type by retained size, and "why is this alive": paths from a GC root as a chain. */
@Component({
  selector: 'imt-heap-instances',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState],
  template: `
    <div class="bar">
      <form class="inline" (submit)="pickType($event, type.value)">
        <label class="inline"><ng-container i18n="@@profiling.heapInstances.type">Type</ng-container>&ngsp;<input #type type="text" aria-label="Type name" i18n-aria-label="@@profiling.heapInstances.typeLabel" [value]="sel.typeName() ?? ''" placeholder="e.g. Shop.Orders.Order" i18n-placeholder="@@profiling.heapInstances.typePlaceholder" /></label>
        <button class="btn" type="submit" i18n="@@profiling.heapInstances.show">Show instances</button>
      </form>
      @if (list.data(); as d) { <span class="muted" i18n="@@profiling.heapInstances.summary">{{ count(d.totalRows) // i18n(ph: "count") }} instances · {{ bytes(d.totalBytes) // i18n(ph: "bytes") }}</span> }
    </div>
    <div class="cols">
      <section class="pane" aria-label="Instances" i18n-aria-label="@@profiling.heapInstances.section">
        @if (!sel.typeName()) {
          <div class="state" role="status" i18n="@@profiling.heapInstances.pick">Pick a type (in the Types tab, or type its name) to list its instances.</div>
        } @else {
          <imt-analysis-state [loading]="list.loading()" [error]="list.error()" [hasData]="!!list.data()" [empty]="list.data()?.totalRows === 0" emptyText="No instances of this type." i18n-emptyText="@@profiling.heapInstances.empty" (retry)="list.retry()">
            <div class="body tight" tabindex="0" role="region" aria-label="Instance rows" i18n-aria-label="@@profiling.heapInstances.rowsRegion">
              <table class="grid" aria-label="Instances by retained size" i18n-aria-label="@@profiling.heapInstances.tableLabel">
                <thead><tr><th scope="col" i18n="@@profiling.heapInstances.col.object">Object</th><th scope="col" class="num" i18n="@@profiling.heapInstances.col.retained">Retained</th><th scope="col" class="num" i18n="@@profiling.heapInstances.col.shallow">Shallow</th></tr></thead>
                <tbody>
                  @for (r of list.data()?.rows ?? []; track r.object) {
                    <tr [class.sel]="sel.object() === r.object">
                      <td><button class="link" type="button" [attr.aria-label]="whyLabel(r.typeName, r.object)" (click)="sel.showPaths(r.object)">&#64;{{ r.object }}</button></td>
                      <td class="num" [title]="exact(r.retained)">{{ bytes(r.retained) }}</td>
                      <td class="num" [title]="exact(r.shallow)">{{ bytes(r.shallow) }}</td>
                    </tr>
                  }
                </tbody>
              </table>
            </div>
          </imt-analysis-state>
        }
      </section>
      <section class="pane paths" tabindex="0" aria-label="Paths to GC roots" i18n-aria-label="@@profiling.heapInstances.pathsSection">
        <h3 class="sec" i18n="@@profiling.heapInstances.why">Why is this alive?</h3>
        @if (sel.object() === null) {
          <div class="state" role="status" i18n="@@profiling.heapInstances.choose">Choose an object to see the references that keep it alive.</div>
        } @else {
          <imt-analysis-state [loading]="paths.loading()" [error]="paths.error()" [hasData]="!!paths.data()" (retry)="paths.retry()">
            @if (paths.data(); as p) {
              @if (p.unreachable) { <p class="muted" role="status" i18n="@@profiling.heapInstances.unreachable">Unreachable: nothing on a path from a GC root keeps it alive (it is garbage).</p> }
              @if (p.truncated) { <p class="muted" role="status" i18n="@@profiling.heapInstances.truncated">More paths exist; the shortest are shown.</p> }
              @for (path of p.paths; track $index; let n = $index) {
                <ol class="chain" [attr.aria-label]="pathLabel(n + 1, path.length)" data-testid="root-path">
                  @for (s of path; track $index; let first = $first; let last = $last) {
                    <li [class.root]="first" [class.target]="last">
                      @if (s.edge) { <span class="edge mono">.{{ s.edge }} →</span> }
                      @if (first) { <span class="tag" i18n="@@profiling.heapInstances.gcRoot">GC root</span> }
                      <span class="node">{{ s.typeName }} <span class="faint mono">&#64;{{ s.object }}</span></span>
                    </li>
                  }
                </ol>
              }
            }
          </imt-analysis-state>
        }
      </section>
    </div>
  `,
  styles: [
    PROFILING_STYLES,
    `
      .tight { padding: 0; }
      .state { padding: 20px 12px; color: var(--muted); text-align: center; }
      .cols { flex: 1; min-height: 0; display: grid; grid-template-columns: minmax(220px, 1fr) 2fr; }
      .pane { min-height: 0; display: flex; flex-direction: column; overflow: auto; }
      .paths { border-left: 1px solid var(--border); padding: 0 12px 12px; }
      .chain { list-style: none; margin: 6px 0 12px; padding: 0; border-left: 2px solid var(--border-3); }
      .chain li { padding: 3px 0 3px 12px; position: relative; display: flex; gap: 6px; align-items: baseline; flex-wrap: wrap; }
      .chain li::before { content: ''; position: absolute; left: -5px; top: 9px; width: 8px; height: 8px; border-radius: 50%; background: var(--border-3); }
      .chain li.root::before { background: var(--accent); }
      .chain li.target { font-weight: 600; color: var(--text); }
      .chain li.target::before { background: var(--lvl-error); }
      .edge { color: var(--muted); }
      .faint { color: var(--faint); }
    `,
  ],
})
export class HeapInstances {
  readonly snapshotId = input.required<number>();
  protected readonly sel = inject(HeapSelection);
  private readonly backend = inject(ProfilingBackend);
  protected readonly list = new AsyncResource<HeapObjectsPage>();
  protected readonly paths = new AsyncResource<HeapPaths>();
  protected readonly count = formatCount;
  protected readonly bytes = formatBytes;
  protected readonly exact = formatExactBytes;

  constructor() {
    effect(() => {
      const snapshotId = this.snapshotId();
      const typeName = this.sel.typeName();
      untracked(() => (typeName ? void this.list.load(() => this.backend.heapInstances({ snapshotId, typeName, limit: 200 })) : this.list.reset()));
    });
    effect(() => {
      const snapshotId = this.snapshotId();
      const object = this.sel.object();
      untracked(() => (object !== null ? void this.paths.load(() => this.backend.heapPaths({ snapshotId, object, maxPaths: 5 })) : this.paths.reset()));
    });
  }

  protected whyLabel(typeName: string, object: number): string {
    return $localize`:@@profiling.heapInstances.whyLabel:Why is ${typeName}:type: @${object}:object: alive?`;
  }

  protected pathLabel(n: number, steps: number): string {
    return $localize`:@@profiling.heapInstances.pathLabel:Path ${n}:n: from a GC root, ${steps}:steps: steps`;
  }

  protected pickType(e: Event, name: string): void {
    e.preventDefault();
    if (name.trim()) this.sel.showInstances(name.trim());
  }
}
