import { inject, Injectable } from '@angular/core';
import { AnalysisResource } from '../../analysis/async-resource';
import { Finding } from '../../core/api.types';
import { EMPTY_QUERY, QueryStore } from '../../core/workspace/query.store';
import { WorkspaceDataStore } from '../../core/workspace/workspace-data.store';
import { ShareBackend } from '../../share/share-backend';

/**
 * The engine's health check (`overview`) of everything opened, grouped like the lanes: one resource shared by Home and
 * the timeline's headline, so showing both asks the engine once. `ensure()` asks for the current data only when it
 * differs from what is loaded (or loading, or failed); `refresh()` always asks again.
 */
@Injectable({ providedIn: 'root' })
export class HealthFindingsStore {
  /** Absent where the sharing feature is not provided: then there are no findings. */
  private readonly share = inject(ShareBackend, { optional: true });
  private readonly data = inject(WorkspaceDataStore);
  private readonly query = inject(QueryStore);
  readonly findings = new AnalysisResource<Finding[]>();
  private lastKey = '';

  /** Whether there is anything to check: a source is open and the sharing feature is there. */
  private available(): boolean {
    return !!this.share && !!this.data.summary()?.sources.length;
  }

  private key(groupBy: readonly string[]): string {
    return JSON.stringify([this.data.dataVersion(), this.data.profileVersion(), groupBy]);
  }

  /** Loads the findings of the current data unless they are loaded or loading; forgets them when nothing is open. */
  ensure(): void {
    if (!this.available()) return this.clear();
    const groupBy = [...this.query.effectiveGroupBy()];
    const key = this.key(groupBy);
    if (key === this.lastKey && !this.findings.error()) return;
    this.start(groupBy, key);
  }

  /** Asks the engine again (Home's Refresh). */
  refresh(): void {
    if (!this.available()) return this.clear();
    const groupBy = [...this.query.effectiveGroupBy()];
    this.start(groupBy, this.key(groupBy));
  }

  clear(): void {
    this.lastKey = '';
    this.findings.reset();
  }

  private start(groupBy: string[], key: string): void {
    this.lastKey = key;
    const share = this.share;
    if (share) void this.findings.load(() => share.overview({ ...EMPTY_QUERY, groupBy }));
  }
}
