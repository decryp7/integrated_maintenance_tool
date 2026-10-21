import { inject, Injectable } from '@angular/core';
import { invoke } from '../core/activity/ipc';
import { Note, NoteChange, NoteDraft, NoteImage, NoteRecord, NotesList, NotesRestore } from '../core/api.types';
import { reportBackendFailures } from '../core/failure-reporter';
import { Logger, withFailureLogging } from '../core/logger';
import { browserMocks } from '../core/mock-registry';
import { isTauri } from '../core/platform';

/**
 * Notes (ADR 0021) as the UI sees them: the engine checks every draft and change, fills event and image anchors,
 * owns the project record and decodes images; the UI names ids and keys. Injected as a class token: the Tauri
 * commands in the app (`app/src-tauri/src/notes/commands.rs`), the in-memory mock in a plain browser. Every method
 * rejects with a `CmdError`.
 */
@Injectable({ providedIn: 'root', useFactory: createNotesBackend })
export abstract class NotesBackend {
  /** The window's notes, each saying whether its anchor still resolves. */
  abstract listNotes(): Promise<NotesList>;
  abstract addNote(draft: NoteDraft): Promise<Note>;
  abstract updateNote(id: string, change: NoteChange): Promise<Note>;
  abstract removeNote(id: string): Promise<boolean>;
  /** The notes as saved in projects and sessions. */
  abstract getNotesRecord(): Promise<NoteRecord>;
  /** Replaces the window's notes with a saved record (null clears them). */
  abstract restoreNotes(record: unknown): Promise<NotesRestore>;
  /** Opens an image file of the workspace (or one the user chose) to mark notes on. */
  abstract openNoteImage(path: string): Promise<NoteImage>;
  /** An image notes mark, by its key: open, or from the project's copy. */
  abstract getNoteImage(key: string): Promise<NoteImage>;
}

/** Talks to the `notes_*` commands. */
export class TauriNotesBackend extends NotesBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }
  listNotes() {
    return this.call<NotesList>('notes_list');
  }
  addNote(draft: NoteDraft) {
    return this.call<Note>('notes_add', { draft });
  }
  updateNote(id: string, change: NoteChange) {
    return this.call<Note>('notes_update', { id, change });
  }
  removeNote(id: string) {
    return this.call<boolean>('notes_remove', { id });
  }
  getNotesRecord() {
    return this.call<NoteRecord>('notes_record');
  }
  restoreNotes(record: unknown) {
    return this.call<NotesRestore>('notes_restore', { record: record ?? null });
  }
  openNoteImage(path: string) {
    return this.call<NoteImage>('notes_image_open', { path });
  }
  getNoteImage(key: string) {
    return this.call<NoteImage>('notes_image_get', { key });
  }
}

export function createNotesBackend(): NotesBackend {
  const backend: NotesBackend = isTauri() ? new TauriNotesBackend() : new (browserMocks().MockNotesBackend)();
  return withFailureLogging<NotesBackend>(backend, inject(Logger), { category: 'ui.notes', onFailure: reportBackendFailures() });
}
