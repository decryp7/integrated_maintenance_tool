import { BackgroundTask, OpenAsk, TaskKind, WindowInfo } from './windowing.types';

/** Event payloads come from the shell; they are checked once here and typed afterwards. */

const KINDS: readonly TaskKind[] = ['liveTail', 'processMonitor', 'perfmon', 'job', 'other'];

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;

export function parseWindows(payload: unknown): WindowInfo[] {
  if (!Array.isArray(payload)) return [];
  return payload
    .filter(isRecord)
    .filter((w) => typeof w['label'] === 'string')
    .map((w) => ({ label: w['label'] as string, title: typeof w['title'] === 'string' ? w['title'] : '', focused: w['focused'] === true, visible: w['visible'] !== false }));
}

export function parseTasks(payload: unknown): BackgroundTask[] {
  if (!Array.isArray(payload)) return [];
  return payload
    .filter(isRecord)
    .filter((t) => typeof t['id'] === 'string' && typeof t['label'] === 'string')
    .map((t) => {
      const progress = typeof t['progress'] === 'number' && Number.isFinite(t['progress']) ? Math.min(1, Math.max(0, t['progress'])) : null;
      return {
        id: t['id'] as string,
        label: t['label'] as string,
        kind: KINDS.includes(t['kind'] as TaskKind) ? (t['kind'] as TaskKind) : 'other',
        window: typeof t['window'] === 'string' ? t['window'] : null,
        progress,
        stoppable: t['stoppable'] === true,
        startedAt: typeof t['startedAt'] === 'number' ? t['startedAt'] : 0,
      };
    });
}

/** Null when the payload is not an open request. */
export function parseOpenAsk(payload: unknown): OpenAsk | null {
  if (!isRecord(payload)) return null;
  const names = Array.isArray(payload['names']) ? payload['names'].filter((n): n is string => typeof n === 'string') : [];
  const count = typeof payload['count'] === 'number' && payload['count'] >= 0 ? Math.floor(payload['count']) : names.length;
  return count > 0 ? { count, names } : null;
}
