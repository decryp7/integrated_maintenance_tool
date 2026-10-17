import { LiveAlert, TailReport } from '../app/core/api.types';
import { EMPTY_QUERY } from '../app/core/workspace.store';
import { emptyTailReport } from '../app/live/live-report';

/** A live change report with the given changes. */
export function report(patch: Partial<TailReport> = {}): TailReport {
  return { ...emptyTailReport(), ...patch };
}

let alertSeq = 0;

/** A live alert (error spike on WEB01 unless patched), with a unique id unless one is given. */
export function alert(patch: Partial<LiveAlert> = {}): LiveAlert {
  return {
    id: `test-alert-${++alertSeq}`,
    kind: 'errorSpike',
    rule: null,
    instance: 'WEB01 · 100',
    severity: 'critical',
    from: 1_000,
    to: 61_000,
    message: 'Error spike in WEB01',
    evidenceQuery: { ...EMPTY_QUERY, from: 1_000, to: 61_000, levels: ['Error', 'Fatal'], groups: ['WEB01 · 100'] },
    ...patch,
  };
}
