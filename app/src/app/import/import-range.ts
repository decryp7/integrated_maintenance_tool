import { inject, Injectable } from '@angular/core';
import { Platform } from '../core/platform';
import { WorkspaceStore } from '../core/workspace.store';
import { DialogService } from '../ui/dialog.service';
import { ImportRangeDialog } from './import-range-dialog';

/** `YYYY-MM-DD`, optionally with `HH:MM[:SS[.fff]]` (a `T` or a space between) and an optional `Z` or `+HH:MM` offset. */
const ISO_TIME = /^(\d{4}-\d{2}-\d{2})(?:[T ](\d{2}:\d{2}(?::\d{2}(?:\.\d{1,3})?)?))?\s*(Z|[+-]\d{2}:\d{2})?$/i;

/**
 * Epoch ms of an ISO-style date and time (UTC unless it carries an offset, like the engine's times), or null when blank
 * or not one: only the unambiguous year-first form is read, never a locale's day/month order.
 */
export function parseRangeTime(text: string): number | null {
  const m = ISO_TIME.exec(text.trim());
  if (!m) return null;
  const ms = Date.parse(`${m[1]}T${m[2] ?? '00:00'}${m[3]?.toUpperCase() ?? 'Z'}`);
  return Number.isFinite(ms) ? ms : null;
}

/** Why a range cannot be used, or null: the engine re-validates; this only saves a round trip. */
export function rangeProblem(from: number | null, to: number | null, rawFrom: string, rawTo: string): string | null {
  if (rawFrom.trim() && from === null) return $localize`:@@import.range.badFrom:The start is not a date and time (write it as 2026-10-14 10:30).`;
  if (rawTo.trim() && to === null) return $localize`:@@import.range.badTo:The end is not a date and time (write it as 2026-10-14 11:00).`;
  if (from !== null && to !== null && to < from) return $localize`:@@import.range.backwards:The end is before the start.`;
  if (from === null && to === null) return $localize`:@@import.range.empty:Give a start, an end, or both.`;
  return null;
}

/** "Open a time range…": pick files or a folder, ask which part of their time to read, import only that. */
@Injectable({ providedIn: 'root' })
export class ImportRange {
  private readonly platform = inject(Platform);
  private readonly workspace = inject(WorkspaceStore);
  private readonly dialogs = inject(DialogService);

  async pickAndOpen(directory: boolean): Promise<void> {
    const paths = await this.platform.pickPaths(directory);
    if (!paths.length) return;
    const range = await this.dialogs.open<{ from: number | null; to: number | null }>(ImportRangeDialog, {
      title: $localize`:@@import.range.title:Open a time range`,
      width: 520,
    }).closed;
    if (range) await this.workspace.openInRange(paths, range);
  }
}
