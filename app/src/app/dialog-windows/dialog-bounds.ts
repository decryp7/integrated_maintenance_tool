import { Rect } from '../dock/dock-geometry';

export const DIALOG_BOUNDS_KEY = 'debrief.dialog.bounds';
export const DIALOG_BOUNDS_VERSION = 1;
/** More remembered dialogs than this are never stored (a corrupt file cannot grow without bound). */
const MAX_REMEMBERED = 200;

function isRect(v: unknown): v is Rect {
  if (typeof v !== 'object' || v === null) return false;
  const r = v as Record<string, unknown>;
  return ['x', 'y', 'width', 'height'].every((k) => typeof r[k] === 'number' && Number.isFinite(r[k])) && (r['width'] as number) > 0 && (r['height'] as number) > 0;
}

/** Remembered window bounds per dialog id, versioned; missing, corrupt or foreign data falls back to nothing. */
export function loadDialogBounds(storage: Pick<Storage, 'getItem'> | null = safeStorage()): Record<string, Rect> {
  try {
    const raw = JSON.parse(storage?.getItem(DIALOG_BOUNDS_KEY) ?? 'null') as { version?: unknown; bounds?: unknown } | null;
    if (!raw || raw.version !== DIALOG_BOUNDS_VERSION || typeof raw.bounds !== 'object' || raw.bounds === null) return {};
    const out: Record<string, Rect> = {};
    for (const [id, r] of Object.entries(raw.bounds as Record<string, unknown>).slice(0, MAX_REMEMBERED)) {
      if (isRect(r)) out[id] = { x: r.x, y: r.y, width: r.width, height: r.height };
    }
    return out;
  } catch {
    return {};
  }
}

/** Remembers `bounds` for dialog `id`. Never throws (a convenience only). */
export function rememberDialogBounds(id: string, bounds: Rect, storage: Pick<Storage, 'getItem' | 'setItem'> | null = safeStorage()): void {
  if (!isRect(bounds)) return;
  try {
    const all = { ...loadDialogBounds(storage), [id]: bounds };
    storage?.setItem(DIALOG_BOUNDS_KEY, JSON.stringify({ version: DIALOG_BOUNDS_VERSION, bounds: all }));
  } catch {
    /* a convenience only */
  }
}

function safeStorage(): Storage | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
}
