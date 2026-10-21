import { inject, Injectable, signal } from '@angular/core';
import { NoteRecord } from '../core/api.types';
import { failureText } from '../core/cmd-error';
import { Logger } from '../core/logger';
import { UiStateContribution } from '../lifecycle/ui-state';
import { NotesBackend } from './notes-backend';

/**
 * The window's notes as part of the project and session UI state (key `notes`, `NoteRecord` v1). The engine owns
 * them; this keeps the last record it returned so saving stays synchronous (a change marks the project unsaved), and
 * hands saved ones back to the engine, which checks them.
 */
@Injectable({ providedIn: 'root' })
export class NotesPersistence {
  private readonly backend = inject(NotesBackend);
  private readonly logger = inject(Logger);
  /** The last record the engine returned (null while there are no notes). */
  readonly record = signal<NoteRecord | null>(null);
  /** Bumped when saved notes were restored, so the notes list reloads. */
  readonly restored = signal(0);
  /** Why saved notes could not be loaded, if so. */
  readonly warning = signal<string | null>(null);

  /** Reads the notes from the engine (after a change). */
  async sync(): Promise<void> {
    try {
      const r = await this.backend.getNotesRecord();
      this.record.set(r.notes.length ? r : null);
    } catch (e) {
      this.logger.warn('notes', 'Notes could not be read for saving', { Error: failureText(e) });
    }
  }

  async restore(value: unknown): Promise<void> {
    try {
      const r = await this.backend.restoreNotes(value ?? null);
      this.warning.set(r.warning);
      await this.sync();
    } catch (e) {
      // The engine could not take them: the saved notes stay in the project untouched (captured back as they were).
      this.record.set(isRecord(value) ? value : null);
      this.logger.warn('notes', 'Saved notes not restored', { Error: failureText(e) });
    }
    this.restored.update((n) => n + 1);
  }

  readonly contribution: UiStateContribution = {
    key: 'notes',
    capture: () => this.record(),
    restore: (v) => void this.restore(v),
    reset: () => void this.restore(null),
  };
}

function isRecord(v: unknown): v is NoteRecord {
  return !!v && typeof v === 'object' && typeof (v as NoteRecord).version === 'number' && Array.isArray((v as NoteRecord).notes);
}
