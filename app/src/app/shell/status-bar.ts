import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { formatCount, tzLabel } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';

@Component({
  selector: 'imt-status-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (store.summary(); as s) {
      <span class="strong">{{ s.sources.length }} source{{ s.sources.length === 1 ? '' : 's' }}</span>
      <span>{{ fmt(s.total) }} events</span>
      @if (store.matchTotal() !== null) {
        <span>{{ fmt(store.matchTotal()!) }} matching</span>
      }
      <span>{{ store.instanceCount() }} instances</span>
      <span>Group: {{ store.effectiveGroupBy().join(', ') || '—' }}</span>
    } @else {
      <span>No workspace</span>
    }
    <span class="spacer"></span>
    <span>Importers: {{ importers() }}</span>
    <button class="tz" (click)="store.setTimezone(store.timezone() === 'utc' ? 'local' : 'utc')" title="Toggle UTC / local time">{{ tz() }}</button>
    <span>IMT 0.1.0</span>
  `,
  styles: `
    :host { height: 24px; flex: none; display: flex; align-items: center; gap: 16px; padding: 0 12px; background: var(--accent); color: var(--accent-ink); font-size: 12px; white-space: nowrap; overflow: hidden; }
    .strong { font-weight: 600; }
    .spacer { flex: 1; }
    .tz { border: 0; background: transparent; color: inherit; cursor: pointer; padding: 0; font-size: 12px; }
    .tz:hover { text-decoration: underline; }
  `,
})
export class StatusBar {
  protected readonly store = inject(WorkspaceStore);
  protected readonly fmt = formatCount;
  protected readonly tz = computed(() => tzLabel(this.store.timezone()));
  protected readonly importers = computed(() => {
    const ids = [...new Set((this.store.summary()?.sources ?? []).map((s) => s.importer))];
    return ids.length ? ids.join(' · ') : '—';
  });
}
