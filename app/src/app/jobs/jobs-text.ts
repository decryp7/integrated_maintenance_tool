import { formatCount } from '../shared/format/units';
import { JobKind, JobLink, JobProgress, JobState } from './jobs.types';

/** Display names of the job kinds (filters, badges). */
export function kindLabel(kind: JobKind): string {
  switch (kind) {
    case 'ai':
      return $localize`:@@jobs.kind.ai:AI`;
    case 'research':
      return $localize`:@@jobs.kind.research:Research`;
    case 'analysis':
      return $localize`:@@jobs.kind.analysis:Analysis`;
    case 'import':
      return $localize`:@@jobs.kind.import:Import`;
    case 'export':
      return $localize`:@@jobs.kind.export:Export`;
    case 'download':
      return $localize`:@@jobs.kind.download:Download`;
    case 'collection':
      return $localize`:@@jobs.kind.collection:Collection`;
    case 'capture':
      return $localize`:@@jobs.kind.capture:Capture`;
    case 'maintenance':
      return $localize`:@@jobs.kind.maintenance:Maintenance`;
  }
}

export function stateLabel(state: JobState): string {
  switch (state) {
    case 'queued':
      return $localize`:@@jobs.state.queued:Queued`;
    case 'running':
      return $localize`:@@jobs.state.running:Running`;
    case 'done':
      return $localize`:@@jobs.state.done:Done`;
    case 'failed':
      return $localize`:@@jobs.state.failed:Failed`;
    case 'cancelled':
      return $localize`:@@jobs.state.cancelled:Cancelled`;
    case 'interrupted':
      return $localize`:@@jobs.state.interrupted:Interrupted`;
  }
}

/** What started the job, in words ("event 42", "query level >= error", the file name). */
export function linkLabel(link: JobLink | null): string | null {
  if (!link) return null;
  switch (link.type) {
    case 'event':
      return $localize`:@@jobs.link.event:event ${link.recordId}:id:`;
    case 'range':
      return $localize`:@@jobs.link.range:a time range`;
    case 'query':
      return $localize`:@@jobs.link.query:query ${link.ql}:ql:`;
    case 'path':
      return link.path.split(/[\\/]/).pop() || link.path;
    case 'chat':
      return $localize`:@@jobs.link.chat:a chat`;
    case 'model':
      return link.id;
  }
}

/** "Queued (position 2)". */
export function queuedLabel(position: number | null): string {
  return position === null ? $localize`:@@jobs.queued.plain:Queued` : $localize`:@@jobs.queued.position:Queued (position ${position}:position:)`;
}

/** The phase label to show next to a job's bar ("Unpacking", "Finding files: 1,234"); empty when the job has no stages. */
export function phaseText(p: JobProgress): string {
  const phase = p.phase;
  if (!phase) return '';
  return phase.scale === 'indeterminate' && p.done !== null ? `${phase.label}: ${formatCount(p.done)}` : phase.label;
}
