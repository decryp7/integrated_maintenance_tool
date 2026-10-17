/** Text of a rejected backend call (Tauri rejects with strings). */
export function errorMessage(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}

const ACCESS_DENIED = /access (is )?denied|requires? (elevat|administrator)|administrator (rights|privileges)|run as administrator|elevation/i;

/** True when a failure was caused by missing rights, so "Restart as administrator" is the next step. */
export function isAccessDenied(message: string | null | undefined): boolean {
  return !!message && ACCESS_DENIED.test(message);
}

/** The last path segment of a Windows or POSIX path. */
export function baseName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
}
