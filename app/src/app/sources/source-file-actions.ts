import { inject, Injectable } from '@angular/core';
import { FileAction, RecordView, SourceInfo } from '../core/api.types';
import { FilesBackend } from '../core/files-backend';
import { describeError } from '../core/logger';
import { WorkspaceStore } from '../core/workspace.store';
import { MenuEntry } from '../ui/context-menu.service';

/** One file action for a source: its label, and why it is unavailable (null = available). */
export interface FileActionState {
  action: FileAction;
  label: string;
  reason: string | null;
}

/**
 * "Open file", "Open at line" and "Show in folder" for an opened source (Explorer, grid rows, inspector). The
 * availability shown here only presents the engine's `fileKind`; the engine resolves the path from the source id
 * and re-checks everything when the action runs.
 */
@Injectable({ providedIn: 'root' })
export class SourceFileActions {
  private readonly backend = inject(FilesBackend);
  private readonly store = inject(WorkspaceStore);

  /** The source of a record, if it is still open. */
  sourceOf(r: RecordView): SourceInfo | null {
    return this.store.summary()?.sources.find((s) => s.id === r.source) ?? null;
  }

  /** The actions for `source`; "Open at line" only when `line` is given (an event, not a file). */
  states(source: SourceInfo | null, line?: number): FileActionState[] {
    const gone = source ? null : $localize`:@@sources.files.reason.gone:The file is no longer open`;
    const kind = source?.fileKind ?? 'text';
    const member = kind === 'archiveMember';
    const inArchive = $localize`:@@sources.files.reason.archive:Inside an archive: extract it to open the file`;
    const out: FileActionState[] = [{ action: 'open', label: $localize`:@@sources.files.open:Open file`, reason: gone ?? (member ? inArchive : null) }];
    if (line !== undefined) {
      const noLines =
        kind === 'compressed'
          ? $localize`:@@sources.files.reason.compressed:Compressed file: its lines are only in Debrief`
          : kind === 'binary'
            ? $localize`:@@sources.files.reason.binary:Not a text file: there is no line to jump to`
            : line < 1
              ? $localize`:@@sources.files.reason.noLine:This event has no line number`
              : null;
      out.push({ action: 'openAtLine', label: $localize`:@@sources.files.openAtLine:Open at line ${line}:line:`, reason: gone ?? (member ? inArchive : noLines) });
    }
    out.push({
      action: 'reveal',
      label: member ? $localize`:@@sources.files.revealArchive:Show archive in folder` : $localize`:@@sources.files.reveal:Show in folder`,
      reason: gone,
    });
    return out;
  }

  /** Context menu entries: unavailable ones disabled, with the reason under the label. */
  items(source: SourceInfo | null, line?: number): MenuEntry[] {
    return this.states(source, line).map((s) => ({
      label: s.label,
      disabled: s.reason !== null,
      ...(s.reason ? { hint: s.reason } : {}),
      run: () => (source ? void this.run(source, s.action, line) : undefined),
    }));
  }

  /** The entries for the record's file at the record's line. */
  itemsForRecord(r: RecordView): MenuEntry[] {
    return this.items(this.sourceOf(r), r.line);
  }

  /** Runs `action`; a refusal or failure shows as the workspace error, a fallback as a notice. */
  async run(source: SourceInfo, action: FileAction, line?: number): Promise<boolean> {
    try {
      const opened = await this.backend.openSourceFile({ sourceId: source.id, action, ...(action === 'openAtLine' && line !== undefined ? { line } : {}) });
      if (action === 'openAtLine' && opened.with === 'defaultApp') {
        this.store.setNotice($localize`:@@sources.files.noEditor:VS Code was not found, so ${source.name}:name: opened with its default program (at the top, not line ${line}:line:).`);
      }
      return true;
    } catch (e) {
      this.store.setError(describeError(e));
      return false;
    }
  }
}
