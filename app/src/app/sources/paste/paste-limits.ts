/** The most text "Paste as log" takes: 8 MiB of UTF-8 (the engine enforces the same cap). */
export const PASTE_MAX_BYTES = 8 * 1024 * 1024;

/** The size of `text` once saved as UTF-8. */
export function byteLength(text: string): number {
  return new TextEncoder().encode(text).length;
}

/** Whether `bytes` is more than a pasted log may hold. */
export function overCap(bytes: number): boolean {
  return bytes > PASTE_MAX_BYTES;
}

/** The name to send: the trimmed field, or null for "let the engine choose". */
export function nameToSend(name: string): string | null {
  const trimmed = name.trim();
  return trimmed ? trimmed : null;
}

const KNOWN_EXTENSIONS = /\.(log|txt|json|jsonl|ndjson|clef|csv|xml|out|err)$/i;

/**
 * The file name the engine makes of a pasted log's name: folders dropped, only letters, digits and `._-() ` kept,
 * `.log` added unless it already ends like a log file. Browser mock only; the engine is the reference.
 */
export function safeLogName(name: string | null): string {
  const leaf = (name ?? '').split(/[\\/]/).pop() ?? '';
  const kept = leaf
    .replace(/[^\p{L}\p{N}._() -]/gu, '')
    .trim()
    .replace(/^\.+/, '');
  const base = kept || 'Pasted log';
  return KNOWN_EXTENSIONS.test(base) ? base : `${base}.log`;
}

/** `name`, or `name (2)`, `name (3)`... before the extension when `taken` has it already. */
export function uniqueName(name: string, taken: ReadonlySet<string>): string {
  if (!taken.has(name)) return name;
  const dot = name.lastIndexOf('.');
  const stem = dot > 0 ? name.slice(0, dot) : name;
  const ext = dot > 0 ? name.slice(dot) : '';
  for (let n = 2; ; n++) {
    const candidate = `${stem} (${n})${ext}`;
    if (!taken.has(candidate)) return candidate;
  }
}
