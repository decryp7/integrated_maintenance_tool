import { ImportProgress } from '../core/api.types';

/** 1536 -> "1.5 KB" (decimal units, as file managers show them). */
export function formatBytes(n: number): string {
  if (n < 1000) return `${n} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let v = n;
  let i = -1;
  do {
    v /= 1000;
    i++;
  } while (v >= 1000 && i < units.length - 1);
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

/** "about 12 s left", "about 3 min left"; empty while unknown. */
export function formatEta(ms: number | null): string {
  if (ms === null || !Number.isFinite(ms) || ms < 0) return '';
  if (ms < 1000) return 'almost done';
  if (ms < 60_000) return `about ${Math.ceil(ms / 1000)} s left`;
  if (ms < 3_600_000) return `about ${Math.ceil(ms / 60_000)} min left`;
  return `about ${(ms / 3_600_000).toFixed(1)} h left`;
}

/** The file name of a path (both separators). */
export function fileName(path: string | null): string {
  if (!path) return '';
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

/** 0–100, from bytes when known, else from files. */
export function percent(p: ImportProgress): number {
  if (p.bytesTotal > 0) return Math.min(100, Math.round((p.bytesDone / p.bytesTotal) * 100));
  if (p.total > 0) return Math.min(100, Math.round((p.done / p.total) * 100));
  return 0;
}

/** "3 of 12 files · 24 MB of 80 MB · about 12 s left" */
export function progressLine(p: ImportProgress): string {
  const parts = [`${p.done} of ${p.total} file${p.total === 1 ? '' : 's'}`];
  if (p.bytesTotal > 0) parts.push(`${formatBytes(p.bytesDone)} of ${formatBytes(p.bytesTotal)}`);
  const eta = formatEta(p.etaMs);
  if (eta) parts.push(eta);
  return parts.join(' · ');
}
