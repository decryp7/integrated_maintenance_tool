import { PluginStatus } from '../core/api.types';
import { ProcessMemory } from '../settings/performance/performance.types';

/** One kind of contribution of a plugin ("importer", "marker", …) with the identifiers it provides. */
export interface ProvideGroup {
  kind: string;
  /** Icon of the Debrief set for the kind. */
  icon: string;
  items: string[];
}

const KIND_ICONS: Record<string, string> = {
  importer: 'file',
  exporter: 'arrowUp',
  marker: 'alert',
  annotator: 'note',
  transform: 'columns',
  hook: 'live',
  service: 'puzzle',
};

/** The part of a contribution before its first colon ("importer:evtx" → "importer"); a bare word is its own kind. */
export function contributionKind(contribution: string): string {
  const at = contribution.indexOf(':');
  return at < 0 ? contribution : contribution.slice(0, at);
}

/** Contributions grouped by kind in first-seen order, e.g. importer 1, marker 2, annotator 2. */
export function provideGroups(contributes: readonly string[]): ProvideGroup[] {
  const groups = new Map<string, ProvideGroup>();
  for (const c of contributes) {
    const kind = contributionKind(c);
    const group = groups.get(kind) ?? { kind, icon: KIND_ICONS[kind] ?? 'puzzle', items: [] };
    group.items.push(c);
    groups.set(kind, group);
  }
  return [...groups.values()];
}

/** How a plugin's memory shows in a table cell. */
export interface MemoryView {
  /** "420 / 1024 MB", or "512 MB cap" before the watcher has reported. */
  text: string;
  /** Used share of the cap, 0 to 100; null when nothing was reported yet. */
  percent: number | null;
  /** `warn` near the cap, `danger` once stopped over it. */
  level: 'ok' | 'warn' | 'danger';
  /** Words shown beside the numbers on exception only. */
  note: string | null;
}

/** The memory cell of a plugin from the memory watcher's row (calm unless the plugin is near or over its cap). */
export function memoryView(row: ProcessMemory | undefined): MemoryView | null {
  if (!row) return null;
  const used = row.usedMb ?? row.peakMb;
  const level = row.state === 'stopped' ? 'danger' : row.state === 'warned' ? 'warn' : 'ok';
  const note =
    level === 'danger' ? $localize`:@@pluginsPanel.memory.stoppedShort:stopped over its cap` : level === 'warn' ? $localize`:@@pluginsPanel.memory.warnedShort:near its cap` : null;
  if (used === null) return { text: $localize`:@@pluginsPanel.memory.capOnly:${row.capMb}:cap: MB cap`, percent: null, level, note };
  const percent = row.capMb > 0 ? Math.min(100, Math.round((used / row.capMb) * 100)) : 0;
  return { text: `${used} / ${row.capMb} MB`, percent, level, note };
}

/** The status word of a plugin; a healthy plugin is shown as a dot only. */
export function statusLabel(status: PluginStatus): string {
  switch (status) {
    case 'active':
      return $localize`:@@pluginsPanel.status.active:Running`;
    case 'disabled':
      return $localize`:@@pluginsPanel.status.disabled:Disabled`;
    case 'quarantined':
      return $localize`:@@pluginsPanel.status.quarantined:Quarantined`;
    case 'inactive':
      return $localize`:@@pluginsPanel.status.inactive:Not running`;
  }
}
