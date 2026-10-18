/**
 * Presentation helpers for the inference runtimes (llama-server builds): plain words for install
 * stages, an entry's state and its devices, and the order the list is shown in. Pure; the engine decides
 * what is installed, usable, suggested and active.
 */
import { AiRuntimeDevice, AiRuntimeEntry, AiRuntimeEvent, AiRuntimeOverview } from '../ai.types';
import { formatBytes, formatEta, formatRate, percent } from '../ai-format';
import { localizeDecimal } from '../../i18n/locale';

export type RuntimeTerminal = Extract<AiRuntimeEvent, { stage: 'ready' | 'failed' | 'cancelled' }>;
export type RuntimeRunning = Exclude<AiRuntimeEvent, RuntimeTerminal>;

export function isTerminal(e: AiRuntimeEvent): e is RuntimeTerminal {
  return e.stage === 'ready' || e.stage === 'failed' || e.stage === 'cancelled';
}

/** What an install is doing, in plain words. */
export function runtimeStageLabel(e: RuntimeRunning, label: string): string {
  switch (e.stage) {
    case 'downloading':
      return e.count > 1
        ? $localize`:@@ai.runtime.stage.downloadingFile:Downloading ${label}:label: (file ${e.index}:index: of ${e.count}:count:)…`
        : $localize`:@@ai.runtime.stage.downloading:Downloading ${label}:label:…`;
    case 'progress':
      return e.verifying
        ? $localize`:@@ai.runtime.stage.verifying:Verifying ${label}:label:…`
        : $localize`:@@ai.runtime.stage.downloading:Downloading ${label}:label:…`;
    case 'unpacking':
      return $localize`:@@ai.runtime.stage.unpacking:Unpacking ${label}:label:…`;
    case 'finishing':
      return $localize`:@@ai.runtime.stage.finishing:Finishing ${label}:label:…`;
  }
}

/** Download progress in percent, or null when the stage has none. */
export function runtimePercent(e: RuntimeRunning | null): number | null {
  return e?.stage === 'progress' ? percent(e.bytes, e.total) : null;
}

/** "12 MB of 32 MB · 24 MB/s · 1 s left". */
export function runtimeProgressText(e: RuntimeRunning | null): string {
  if (e?.stage !== 'progress') return '';
  const base = $localize`:@@ai.runtime.progress.of:${formatBytes(e.bytes)}:done: of ${formatBytes(e.total)}:total:`;
  if (e.verifying) return base;
  const eta = formatEta(e.etaSecs);
  return [base, e.bytesPerSec > 0 ? formatRate(e.bytesPerSec) : '', eta && $localize`:@@ai.runtime.progress.left:${eta}:eta: left`]
    .filter(Boolean)
    .join(' · ');
}

export type EntryState = 'active' | 'ready' | 'problem' | 'unchecked' | 'available';

/** Where a build stands on this computer. */
export function entryState(e: AiRuntimeEntry): EntryState {
  if (!e.installed) return 'available';
  if (e.active) return 'active';
  if (e.usable === false) return 'problem';
  return e.usable === null ? 'unchecked' : 'ready';
}

export function entryStateLabel(e: AiRuntimeEntry): string {
  switch (entryState(e)) {
    case 'active':
      return $localize`:@@ai.runtime.state.active:In use`;
    case 'ready':
      return $localize`:@@ai.runtime.state.ready:Installed, works here`;
    case 'problem':
      return $localize`:@@ai.runtime.state.problem:Installed, does not work here`;
    case 'unchecked':
      return $localize`:@@ai.runtime.state.unchecked:Installed`;
    case 'available':
      return $localize`:@@ai.runtime.state.available:Not installed`;
  }
}

/** "NVIDIA GeForce RTX 4060 (CUDA0, 7.4 of 8.0 GB free)". */
export function deviceLine(d: AiRuntimeDevice): string {
  const gb = (mib: number) => localizeDecimal((mib / 1024).toFixed(1));
  return $localize`:@@ai.runtime.device:${d.description}:description: (${d.name}:name:, ${gb(d.freeMib)}:free: of ${gb(d.totalMib)}:total: GB free)`;

}

/** The suggested build, when the user should be offered it first (nothing usable is installed yet). */
export function suggestedDownload(o: AiRuntimeOverview | null): AiRuntimeEntry | null {
  if (!o || o.customPath) return null;
  const s = o.builds.find((b) => b.suggested && !b.installed);
  if (!s) return null;
  const working = o.builds.some((b) => b.installed && b.usable !== false);
  return o.needsDownload || !working ? s : null;
}

/** The list order: the suggested build first, then installed ones, then the rest (catalog order). */
export function orderedBuilds(o: AiRuntimeOverview | null): AiRuntimeEntry[] {
  if (!o) return [];
  const rank = (b: AiRuntimeEntry) => (b.suggested ? 0 : b.installed ? 1 : 2);
  return [...o.builds].sort((a, b) => rank(a) - rank(b));
}
