import { ImportProgress } from '../core/api.types';
import { plural } from '../i18n/locale';
import { formatBytes, formatCount, formatEta } from '../shared/format/units';

export { formatBytes };

/** The file name of a path (both separators). */
export function fileName(path: string | null): string {
  if (!path) return '';
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

/** 0–100, from bytes when known, else from files; 0 while the files are still being found (the bar is then indeterminate). */
export function percent(p: ImportProgress): number {
  if (p.phase === 'finding') return 0;
  if (p.phase === 'finishing') return 100;
  if (p.phase === 'detecting') return p.total > 0 ? Math.min(100, Math.round((p.done / p.total) * 100)) : 0;
  if (p.bytesTotal > 0) return Math.min(100, Math.round((p.bytesDone / p.bytesTotal) * 100));
  if (p.total > 0) return Math.min(100, Math.round((p.done / p.total) * 100));
  return 0;
}

/** "Finding files… 12,345 found (3.1 GB)": the count grows while the folders are walked; the total is not known yet. */
export function findingText(p: ImportProgress): string {
  const count = formatCount(p.done);
  const size = formatBytes(p.bytesTotal);
  return $localize`:@@import.format.finding:Finding files… ${count}:count: found (${size}:size:)`;
}

/** "Detecting formats… 4,000 of 12,345" */
export function detectingText(p: ImportProgress): string {
  const done = formatCount(p.done);
  const total = formatCount(p.total);
  return $localize`:@@import.format.detecting:Detecting formats… ${done}:done: of ${total}:total:`;
}

/** What the import works on now, for the card: the folder being listed, or the file being read. */
export function detailText(p: ImportProgress): string {
  const name = fileName(p.current);
  if (!name) return '';
  return p.phase === 'finding' ? $localize`:@@import.format.inFolder:in ${name}:folder:` : $localize`:@@import.format.reading:Reading ${name}:file:`;
}

/** "3 of 12 files · 24 MB of 80 MB · about 12 s left" */
export function progressLine(p: ImportProgress): string {
  if (p.phase === 'finding') return findingText(p);
  if (p.phase === 'detecting') return detectingText(p);
  const done = formatCount(p.done);
  const total = formatCount(p.total);
  const parts = [
    plural(p.total, {
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
  if (p.phase === 'finishing') return p.stage ?? $localize`:@@import.format.finishing:Finishing…`;
  if (p.phase !== 'importing') return progressLine(p);
  if (p.total <= 0) return $localize`:@@import.format.scanning:Scanning for files…`;
  const line = progressLine(p);
  return $localize`:@@import.format.importing:Importing ${line}:line:`;
}
