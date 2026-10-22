import { Note, NoteAnchor, NoteChange, NoteDraft, NoteImage, NoteRecord, NoteRegion, NotesList, NotesRestore } from '../../core/api.types';
import { CmdError } from '../../core/cmd-error';
import type { NotesBackend } from '../notes-backend';

const MAX_TEXT_BYTES = 16 * 1024;
const MAX_REGIONS = 16;

const invalid = (message: string) => new CmdError({ code: 'invalid', message, retryable: false });
const notFound = (message: string) => new CmdError({ code: 'notFound', message, retryable: false });

/** A 64-hex key from a string (the engine hashes the file's bytes; any stable key will do here). */
export function mockImageKey(s: string): string {
  let h = 0x811c9dc5;
  const out: string[] = [];
  for (let round = 0; round < 8; round++) {
    for (const c of `${round}:${s}`) h = Math.imul(h ^ c.charCodeAt(0), 0x01000193) >>> 0;
    out.push(h.toString(16).padStart(8, '0'));
  }
  return out.join('');
}

/** A made-up screenshot: a dark window with a red error banner and a few rows (same shape as a real capture). */
function screenshot(width: number, height: number): string {
  const rows = [0, 1, 2, 3, 4].map((i) => `<rect x="40" y="${150 + i * 50}" width="${width - 80}" height="34" fill="#2a2f3a"/>`).join('');
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}"><rect width="100%" height="100%" fill="#1b1e24"/><rect x="40" y="40" width="${width - 80}" height="70" fill="#b3261e"/>${rows}</svg>`;
  return `data:image/svg+xml;base64,${btoa(svg)}`;
}

function fits(r: NoteRegion, w: number, h: number): boolean {
  return r.w > 0 && r.h > 0 && r.x >= 0 && r.y >= 0 && r.x + r.w <= w && r.y + r.h <= h;
}

/**
 * In-memory notes shaped like the engine's (browser development and UI tests only; the engine is the reference for
 * the checks and fills anchors from the store: here the draft's own event fields are kept).
 */
export class MockNotesBackend implements NotesBackend {
  private notes: Note[] = [];
  private next = 1;
  private readonly images = new Map<string, NoteImage>();

  private checkText(text: string): string {
    if (new TextEncoder().encode(text).length > MAX_TEXT_BYTES) throw new CmdError({ code: 'limit', message: 'the note is too long', retryable: false });
    const t = text.replace(/\r\n?/g, '\n').trimEnd();
    if (!t.trim()) throw invalid('write something in the note');
    return t;
  }

  private checkRegions(regions: NoteRegion[], w: number, h: number): NoteRegion[] {
    if (!regions.length) throw invalid('mark at least one area of the image');
    if (regions.length > MAX_REGIONS) throw invalid(`mark at most ${MAX_REGIONS} areas on one note`);
    if (!regions.every((r) => fits(r, w, h))) throw invalid('the area is not inside the image');
    return regions.map((r) => ({ ...r }));
  }

  private checkAnchor(a: NoteAnchor): NoteAnchor {
    if (a.kind === 'range' && a.from >= a.to) throw invalid('the range must end after it starts');
    if (a.kind === 'image') {
      const img = this.images.get(a.image.key);
      if (!img) throw notFound('the image is no longer open: open it again');
      return { kind: 'image', image: { key: img.key, name: img.name, path: img.path, width: img.width, height: img.height }, regions: this.checkRegions(a.regions, img.width, img.height) };
    }
    return structuredClone(a);
  }

  private tags(tags: string[] | undefined): string[] {
    return [...new Set((tags ?? []).map((t) => t.trim().replace(/^#/, '').toLowerCase()).filter(Boolean))];
  }

  private find(id: string): Note {
    const n = this.notes.find((x) => x.id === id);
    if (!n) throw notFound('this note no longer exists');
    return n;
  }

  async listNotes(): Promise<NotesList> {
    return { notes: this.notes.map((n) => ({ ...structuredClone(n), resolved: true })), maxNotes: 5000, maxTextBytes: MAX_TEXT_BYTES, maxTags: 8, maxRegions: MAX_REGIONS };
  }

  async addNote(draft: NoteDraft): Promise<Note> {
    const now = Date.now();
    const anchor = this.checkAnchor(draft.anchor);
    const text = this.checkText(draft.text);
    const note: Note = { id: `n${this.next++}`, anchor, text, tags: this.tags(draft.tags), severity: draft.severity ?? 'info', author: 'you', createdMs: now, updatedMs: now };
    this.notes.push(note);
    return structuredClone(note);
  }

  async updateNote(id: string, change: NoteChange): Promise<Note> {
    const n = this.find(id);
    const text = change.text !== undefined ? this.checkText(change.text) : n.text;
    let anchor = n.anchor;
    if (change.regions !== undefined) {
      if (anchor.kind !== 'image') throw invalid('only notes on images have areas');
      anchor = { ...anchor, regions: this.checkRegions(change.regions, anchor.image.width, anchor.image.height) };
    }
    Object.assign(n, { text, anchor, tags: change.tags !== undefined ? this.tags(change.tags) : n.tags, severity: change.severity ?? n.severity, updatedMs: Date.now() });
    return structuredClone(n);
  }

  async removeNote(id: string): Promise<boolean> {
    this.find(id);
    this.notes = this.notes.filter((n) => n.id !== id);
    return true;
  }

  async getNotesRecord(): Promise<NoteRecord> {
    return { version: 1, notes: structuredClone(this.notes) };
  }

  async restoreNotes(record: unknown): Promise<NotesRestore> {
    const r = record as Partial<NoteRecord> | null;
    if (r == null) {
      this.notes = [];
      return { count: 0, warning: null };
    }
    if (typeof r.version !== 'number' || !Array.isArray(r.notes)) {
      this.notes = [];
      return { count: 0, warning: "The project's notes are damaged and were not loaded." };
    }
    this.notes = structuredClone(r.notes.filter((n) => n && typeof n.id === 'string' && typeof n.text === 'string' && n.anchor));
    this.next = Math.max(0, ...this.notes.map((n) => Number(n.id.slice(1)) || 0)) + 1;
    return { count: this.notes.length, warning: null };
  }

  async openNoteImage(path: string): Promise<NoteImage> {
    const width = 800;
    const height = 450;
    const name = path.split(/[\\/]/).at(-1) ?? path;
    const img: NoteImage = { key: mockImageKey(path), name, path, width, height, viewWidth: width, viewHeight: height, dataUrl: screenshot(width, height) };
    this.images.set(img.key, img);
    return structuredClone(img);
  }

  async getNoteImage(key: string): Promise<NoteImage> {
    const img = this.images.get(key);
    if (!img) throw notFound('no note marks this image');
    return structuredClone(img);
  }
}
