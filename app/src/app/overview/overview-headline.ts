import { ChangeDetectionStrategy, Component, computed, effect, inject, Injectable, Injector, signal, untracked } from '@angular/core';
import { Finding } from '../core/api.types';
import { formatCount } from '../core/format';
import { ConsumerGate } from '../core/panel-visibility';
import { EMPTY_QUERY, QueryStore } from '../core/workspace/query.store';
import { WorkspaceDataStore } from '../core/workspace/workspace-data.store';
import { DockService } from '../dock/dock.service';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { plural } from '../i18n/locale';
import { Icon } from '../shared/icon';
import { ShareBackend } from '../share/share-backend';
import { OverviewActions } from './overview-actions';
import { OVERVIEW_VIEW } from './overview.plugin';

/** The headline numbers of the health check: what the timeline header points at. */
export interface Headline {
  crashes: readonly Finding[];
  errors: number;
  silent: readonly Finding[];
}

/** The headline of the engine's findings and the summary's level counts; null when nothing is notable. */
export function headlineOf(findings: readonly Finding[], levels: Partial<Record<string, number>> | null): Headline | null {
  const crashes = findings.filter((f) => f.kind === 'crash');
  const silent = findings.filter((f) => f.kind === 'silence');
  const errors = (levels?.['Error'] ?? 0) + (levels?.['Fatal'] ?? 0);
  return crashes.length || silent.length || errors ? { crashes, errors, silent } : null;
}

/**
 * The health check's findings (the engine's `overview`, the same request as the Overview view) for the headline in
 * the timeline header. Asked only while a headline is mounted and visible, and again when the data or the grouping
 * changes; a failure just hides the headline (the Overview view reports it).
 */
@Injectable({ providedIn: 'root' })
export class OverviewHeadlineStore {
  /** Absent where the sharing feature is not provided: then there is no headline. */
  private readonly share = inject(ShareBackend, { optional: true });
  private readonly data = inject(WorkspaceDataStore);
  private readonly query = inject(QueryStore);
  readonly gate = new ConsumerGate();
  private readonly found = signal<readonly Finding[]>([]);
  private seq = 0;
  private lastKey = '';

  readonly headline = computed(() => headlineOf(this.found(), this.data.summary()?.levels ?? null));

  constructor() {
    effect(() => {
      if (!this.gate.active()) return;
      const summary = this.data.summary();
      const version = this.data.dataVersion();
      const groupBy = this.query.effectiveGroupBy();
      untracked(() => {
        if (!summary?.sources.length || !this.share) {
          this.found.set([]);
          this.lastKey = '';
          return;
        }
        const key = JSON.stringify([version, groupBy]);
        if (key === this.lastKey) return;
        this.lastKey = key;
        const mine = ++this.seq;
        this.share
          .overview({ ...EMPTY_QUERY, groupBy })
          .then((f) => mine === this.seq && this.found.set(f))
          .catch(() => {
            if (mine !== this.seq) return;
            this.found.set([]);
            this.lastKey = '';
          });
      });
    });
  }
}

/**
 * One compact line in the timeline header with the health check's headline, e.g. "⚠ 2 crashes · 287 errors · 1 went
 * silent — Review": each count is a button (jump to the next crash, show the errors, show the next silence) and
 * Review opens the Overview. Hidden when nothing is notable and while the Overview itself is open.
 */
@Component({
  selector: 'imt-overview-headline',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  host: { style: 'display: contents' },
  template: `
    @if (shown(); as h) {
      <div class="headline" role="toolbar" aria-label="Health check" i18n-aria-label="@@overview.headline.aria" data-testid="overview-headline">
        <imt-icon name="alert" [size]="13" class="warn" />
        @if (h.crashes.length) {
          <button type="button" class="count bad" data-testid="headline-crashes" [title]="crashTitle" (click)="next('crash', h.crashes)">{{ crashesText(h.crashes.length) }}</button>
        }
        @if (h.errors) {
          <button type="button" class="count bad" data-testid="headline-errors" [title]="errorsTitle" (click)="showErrors()">{{ errorsText(h.errors) }}</button>
        }
        @if (h.silent.length) {
          <button type="button" class="count" data-testid="headline-silent" [title]="silentTitle" (click)="next('silence', h.silent)">{{ silentText(h.silent.length) }}</button>
        }
        <span class="faint" aria-hidden="true">—</span>
        <button type="button" class="review" data-testid="headline-review" (click)="review()" i18n="@@overview.headline.review">Review</button>
      </div>
    }
  `,
  styles: `
    .headline { flex: none; display: flex; align-items: center; gap: 4px; font-size: var(--fs-sm); color: var(--text-2); white-space: nowrap; }
    .warn { color: var(--warn-text); flex: none; }
    /* 24px targets (WCAG 2.5.8) on a compact line. */
    .count, .review { min-height: 24px; border: 0; background: transparent; padding: 0 4px; border-radius: 4px; font: inherit; color: inherit; cursor: pointer; }
    .count + .count::before { content: '· '; color: var(--faint); }
    .count.bad { color: var(--danger-text); }
    .review { color: var(--chip-text); text-decoration: underline; }
    .count:hover, .review:hover { background: var(--bg-hover); }
    .count:focus-visible, .review:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 1px; }
  `,
})
export class OverviewHeadline {
  private readonly store = inject(OverviewHeadlineStore);
  private readonly dock = inject(DockService);
  /** The Overview's own actions, resolved when used (they need the analysis backend). */
  private readonly injector = inject(Injector);
  private readonly ctx = inject(COMMAND_CONTEXT);
  /** Which crash or silence the next click goes to (each click moves on). */
  private readonly cursor = new Map<string, number>();

  protected readonly crashTitle = $localize`:@@overview.headline.crashTitle:Jump to the next crash`;
  protected readonly errorsTitle = $localize`:@@overview.headline.errorsTitle:Show the errors and fatal events`;
  protected readonly silentTitle = $localize`:@@overview.headline.silentTitle:Show the next time a source went silent`;

  /** The headline, unless the Overview itself is open. */
  protected readonly shown = computed(() => (this.dock.isOpen(OVERVIEW_VIEW) ? null : this.store.headline()));

  constructor() {
    this.store.gate.use();
  }

  protected crashesText(n: number): string {
    return plural(n, { one: $localize`:@@overview.headline.crashes.one:${n}:count: crash`, other: $localize`:@@overview.headline.crashes.other:${n}:count: crashes` });
  }

  protected errorsText(n: number): string {
    const count = formatCount(n);
    return plural(n, { one: $localize`:@@overview.headline.errors.one:${count}:count: error`, other: $localize`:@@overview.headline.errors.other:${count}:count: errors` });
  }

  protected silentText(n: number): string {
    return plural(n, { one: $localize`:@@overview.headline.silent.one:${n}:count: went silent`, other: $localize`:@@overview.headline.silent.other:${n}:count: went silent` });
  }

  /** Crashes jump to the event; silences show their range. Each click goes to the next one. */
  protected next(kind: 'crash' | 'silence', findings: readonly Finding[]): void {
    const i = (this.cursor.get(kind) ?? -1) + 1;
    const index = i % findings.length;
    this.cursor.set(kind, index);
    const finding = findings[index];
    if (finding) void this.injector.get(OverviewActions).run(finding, kind === 'crash' ? 'jump' : 'showRange');
  }

  protected showErrors(): void {
    this.ctx.setQuery({ levels: ['Error', 'Fatal'] });
    this.ctx.showView('events');
  }

  protected review(): void {
    this.ctx.showView(OVERVIEW_VIEW);
  }
}
