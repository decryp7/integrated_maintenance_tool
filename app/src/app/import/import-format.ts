import { ImportProgress } from '../core/api.types';
import { localizeDecimal, plural } from '../i18n/locale';

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
  return `${localizeDecimal(v < 10 ? v.toFixed(1) : Math.round(v))} ${units[i]}`;
}

/** "about 12 s left", "about 3 min left"; empty while unknown. */
export function formatEta(ms: number | null): string {
  if (ms === null || !Number.isFinite(ms) || ms < 0) return '';
  if (ms < 1000) return $localize`:@@import.format.eta.almostDone:almost done`;
  if (ms < 60_000) {
    const s = Math.ceil(ms / 1000);
    return $localize`:@@import.format.eta.seconds:about ${s}:seconds: s left`;
  }
  if (ms < 3_600_000) {
    const min = Math.ceil(ms / 60_000);
    return $localize`:@@import.format.eta.minutes:about ${min}:minutes: min left`;
  }
  const h = localizeDecimal((ms / 3_600_000).toFixed(1));
  return $localize`:@@import.format.eta.hours:about ${h}:hours: h left`;
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
  const { done, total } = p;
  const parts = [
    plural(total, {
      one: $localize`:@@import.format.files.one:${done}:done: of ${total}:total: file`,
      other: $localize`:@@import.format.files.other:${done}:done: of ${total}:total: files`,
    }),
  ];
  if (p.bytesTotal > 0) {
    const doneBytes = formatBytes(p.bytesDone);
    const totalBytes = formatBytes(p.bytesTotal);
    parts.push($localize`:@@import.format.bytes:${doneBytes}:done: of ${totalBytes}:total:`);
  }
  const eta = formatEta(p.etaMs);
  if (eta) parts.push(eta);
  return parts.join(' · ');
}
