import type { RecordView } from './api.types';

/** The drag-and-drop type for an event dragged from the events grid onto another view. */
export const EVENT_DRAG_TYPE = 'application/x-debrief-event';

/** Longest message carried by a drag; the rest is cut (a drop only needs the first line). */
const MAX_EXCERPT = 300;

/** What a dragged event carries: enough for a view to refer to it (the engine resolves the id). */
export interface EventDrag {
  id: number;
  ts: number | null;
  source: string;
  excerpt: string;
}

/** The first line of the message, capped. */
function excerptOf(message: string): string {
  return (message.split('\n')[0] ?? '').slice(0, MAX_EXCERPT);
}

/** Puts an event on the drag (also as plain text, so it can be dropped into a text field). */
export function writeEventDrag(dt: DataTransfer, record: Pick<RecordView, 'id' | 'ts' | 'sourceName' | 'message'>): void {
  const drag: EventDrag = { id: record.id, ts: record.ts, source: record.sourceName, excerpt: excerptOf(record.message) };
  dt.setData(EVENT_DRAG_TYPE, JSON.stringify(drag));
  dt.setData('text/plain', record.message);
  dt.effectAllowed = 'copy';
}

/** Is an event being dragged? (Only the types are readable while dragging over a target.) */
export function hasEventDrag(dt: DataTransfer | null): boolean {
  return !!dt && Array.from(dt.types ?? []).includes(EVENT_DRAG_TYPE);
}

/** Reads a dropped event; null when the drop carries none or malformed data (never trusted). */
export function readEventDrag(dt: DataTransfer | null): EventDrag | null {
  if (!dt) return null;
  try {
    const raw: unknown = JSON.parse(dt.getData(EVENT_DRAG_TYPE));
    const v = raw as Partial<EventDrag> | null;
    if (!v || typeof v !== 'object' || typeof v.id !== 'number' || !Number.isSafeInteger(v.id) || v.id < 0) return null;
    return {
      id: v.id,
      ts: typeof v.ts === 'number' && Number.isFinite(v.ts) ? v.ts : null,
      source: typeof v.source === 'string' ? v.source.slice(0, 1000) : '',
      excerpt: typeof v.excerpt === 'string' ? v.excerpt.slice(0, MAX_EXCERPT) : '',
    };
  } catch {
    return null;
  }
}
