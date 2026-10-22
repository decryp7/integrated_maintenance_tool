import { inject, Injectable } from '@angular/core';
import { Platform } from '../core/platform';
import { WorkspaceStore } from '../core/workspace.store';
import { DialogService } from '../ui/dialog.service';
import { ImportRangeDialog } from './import-range-dialog';

/** Epoch ms of a `datetime-local` text (read as UTC, like the engine's times), or null when blank or not a time. */
export function parseRangeTime(text: string): number | null {
  const t = text.trim();
  if (!t) return null;
  const ms = Date.parse(/Z$|[+-]\d{2}:\d{2}$/.test(t) ? t : `${t}Z`);
  return Number.isFinite(ms) ? ms : null;
}

/** Why a range cannot be used, or null: the engine re-validates; this only saves a round trip. */
export function rangeProblem(from: number | null, to: number | null, rawFrom: string, rawTo: string): string | null {
  if (rawFrom.trim() && from === null) return $localize`:@@import.range.badFrom:The start is not a date and time.`;
  if (rawTo.trim() && to === null) return $localize`:@@import.range.badTo:The end is not a date and time.`;
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
