export const DIALOG_BOUNDS_KEY = 'debrief.dialog.bounds';
/** 2: an offset from the parent window plus a size (1 held absolute screen bounds, which pulled dialogs onto other displays). */
export const DIALOG_BOUNDS_VERSION = 2;
/** More remembered dialogs than this are never stored (a corrupt file cannot grow without bound). */
const MAX_REMEMBERED = 200;

/**
 * What is remembered of a dialog window (logical px): its inner size and the offset of its top-left corner from its parent
 * window's. Never an absolute screen position: the shell decides where the dialog goes, always on the parent's monitor
 * (ADR 0054, placement policy), and uses the offset only when the whole dialog then stays on that monitor.
 */
export interface RememberedPlace {
  dx: number;
  dy: number;
  width: number;
  height: number;
}

export function isRememberedPlace(v: unknown): v is RememberedPlace {
  if (typeof v !== 'object' || v === null) return false;
  const r = v as Record<string, unknown>;
  return ['dx', 'dy', 'width', 'height'].every((k) => typeof r[k] === 'number' && Number.isFinite(r[k])) && (r['width'] as number) > 0 && (r['height'] as number) > 0;
}

/** Remembered places per dialog id, versioned; missing, corrupt, old or foreign data falls back to nothing. */
export function loadDialogBounds(storage: Pick<Storage, 'getItem'> | null = safeStorage()): Record<string, RememberedPlace> {
  try {
    const raw = JSON.parse(storage?.getItem(DIALOG_BOUNDS_KEY) ?? 'null') as { version?: unknown; bounds?: unknown } | null;
    if (!raw || raw.version !== DIALOG_BOUNDS_VERSION || typeof raw.bounds !== 'object' || raw.bounds === null) return {};
    const out: Record<string, RememberedPlace> = {};
    for (const [id, r] of Object.entries(raw.bounds as Record<string, unknown>).slice(0, MAX_REMEMBERED)) {
      if (isRememberedPlace(r)) out[id] = { dx: r.dx, dy: r.dy, width: r.width, height: r.height };
    }
    return out;
  } catch {
    return {};
  }
}

/** Remembers `place` for dialog `id`. Never throws (a convenience only). */
export function rememberDialogBounds(id: string, place: RememberedPlace, storage: Pick<Storage, 'getItem' | 'setItem'> | null = safeStorage()): void {
  if (!isRememberedPlace(place)) return;
  try {
    const all = { ...loadDialogBounds(storage), [id]: place };
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
