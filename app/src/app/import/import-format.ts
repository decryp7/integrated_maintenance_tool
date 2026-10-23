import { ImportProgress } from '../core/api.types';
import { plural } from '../i18n/locale';
import { formatBytes, formatEta } from '../shared/format/units';

export { formatBytes };

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
  const eta = p.etaMs === null ? '' : formatEta(p.etaMs / 1000);
  if (eta) parts.push(eta);
  return parts.join(' · ');
}

/**
 * The status-bar sentence: "Importing 3 of 12 files · …", or "Scanning for files…" while the engine has not counted
 * them yet (a folder is still being walked), instead of a "0 of 0 files" that reads as broken.
 */
export function importStatusText(p: ImportProgress): string {
  if (p.phase) return p.phase;
  if (p.total <= 0) return $localize`:@@import.format.scanning:Scanning for files…`;
  const line = progressLine(p);
  return $localize`:@@import.format.importing:Importing ${line}:line:`;
}
