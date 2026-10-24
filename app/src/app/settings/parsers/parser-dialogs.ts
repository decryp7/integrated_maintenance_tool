import { inject, Injectable } from '@angular/core';
import { ParserDocDto, ParserEntryDto, ParserImportCheckDto, ParserLibraryDto } from '../../core/api.types';
import { ConfirmDialog } from '../../ui/confirm-dialog';
import { DialogService } from '../../ui/dialog.service';
import { ParserEditor } from './parser-editor';
import { ParserImportConfirm } from './parser-import-confirm';

/** The questions and editors of Settings ▸ Parsers, as dialogs (the store asks; tests answer through a fake). */
@Injectable({ providedIn: 'root' })
export class ParserDialogs {
  private readonly dialogs = inject(DialogService);

  /** "Remove …?": true only for an explicit Remove. */
  async confirmRemove(entry: ParserEntryDto): Promise<boolean> {
    const name = entry.name;
    const message =
      entry.origin === 'override'
        ? $localize`:@@settings.parsers.remove.override:Remove your override of ${name}:name:? The built-in parser is used again.`
        : $localize`:@@settings.parsers.remove.user:Remove the parser ${name}:name:? Its file is deleted; files it read are read by another parser when they are read again.`;
    const ref = this.dialogs.open<boolean>(ConfirmDialog, {
      title: $localize`:@@settings.parsers.remove.title:Remove parser?`,
      width: 460,
      inputs: { message, detail: entry.id, confirmLabel: $localize`:@@settings.parsers.remove.confirm:Remove`, danger: true },
    });
    return (await ref.closed) === true;
  }

  /** Importing a parser whose id is in the library: versions and diff, then Update/Replace. */
  async confirmImport(check: ParserImportCheckDto): Promise<boolean> {
    const ref = this.dialogs.open(ParserImportConfirm, {
      title: check.relation === 'newer' ? $localize`:@@settings.parsers.import.titleUpdate:Update parser` : $localize`:@@settings.parsers.import.titleReplace:Replace parser`,
      width: 720,
      inputs: { check },
    });
    return (await ref.closed) === true;
  }

  /** Opens the TOML editor; resolves with the library after a save, or undefined. */
  async edit(doc: ParserDocDto, options: { isNew?: boolean; readSources?: number[] } = {}): Promise<ParserLibraryDto | undefined> {
    const ref = this.dialogs.open(ParserEditor, {
      title: options.isNew ? $localize`:@@settings.parsers.editor.titleNew:New parser` : $localize`:@@settings.parsers.editor.title:Edit parser ${doc.id}:id:`,
      width: 900,
      inputs: { doc, isNew: options.isNew ?? false, readSources: options.readSources ?? [] },
    });
    return ref.closed;
  }
}
