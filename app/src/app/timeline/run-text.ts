/** Localised words for runs between lifecycle markers (statuses and anomalies), shared by the timeline, the Runs view and the editor. */
import { RunInfo, RunStatus } from '../core/api.types';
import { formatDuration } from '../core/format';
import { anomalyOf } from './run-layout';

export function runStatusLabel(status: RunStatus): string {
  switch (status) {
    case 'closed':
      return $localize`:Run status|@@runs.status.closed:Stopped`;
    case 'open':
      return $localize`:Run status|@@runs.status.open:Still running`;
    case 'noStop':
      return $localize`:Run status|@@runs.status.noStop:No stop seen`;
    case 'noStart':
      return $localize`:Run status|@@runs.status.noStart:Stop without start`;
  }
}

/** Why a run is flagged, in a sentence; null when it is not. */
export function runAnomalyText(run: RunInfo): string | null {
  switch (anomalyOf(run)) {
    case 'noStart':
      return $localize`:@@runs.anomaly.noStart:A stop was logged without a start before it.`;
    case 'noStop':
      return $localize`:@@runs.anomaly.noStop:Started again before a stop was logged — likely a crash or restart.`;
    case 'tooLong':
      return $localize`:@@runs.anomaly.tooLong:Ran longer than the rule's maximum duration.`;
    default:
      return null;
  }
}

/** "12m 30s", or "–" for a stop without start. */
export function runDuration(run: RunInfo): string {
  return run.status === 'noStart' ? '–' : formatDuration(run.durationMs);
}
