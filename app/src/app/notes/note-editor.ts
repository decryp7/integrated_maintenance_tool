import { ChangeDetectionStrategy, Component, computed, inject, input, OnInit, signal } from '@angular/core';
import { NoteAnchor, NoteItem, NoteSeverity } from '../core/api.types';
import { MarkdownView } from '../share/report/markdown-view';
import { DIALOG_REF } from '../ui/dialog.service';
import { anchorText, kindLabel, severityLabel } from './notes-model';
import { NotesStore } from './notes.store';

const encoder = new TextEncoder();
const SEVERITIES: readonly NoteSeverity[] = ['info', 'warning', 'problem'];

/** Tags as typed: separated by commas or spaces, `#` optional (the engine normalises and checks them). */
export function parseTags(text: string): string[] {
  return text
    .split(/[\s,]+/)
    .map((t) => t.replace(/^#/, '').trim())
    .filter(Boolean);
}

/**
 * Writes a new note on an anchor or edits one: Markdown text with a preview (text nodes only, never HTML), tags and
 * a severity. Saving goes through the engine, which checks everything; the dialog stays open with the reason when it
 * refuses. Closes with the note's id once saved.
 */
@Component({
  selector: 'debrief-note-editor',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [MarkdownView],
  template: `
    <p class="anchor" data-testid="note-anchor"><span class="kind">{{ kind() }}</span> {{ about() }}</p>
    <div class="tabs" role="tablist" aria-label="Editor mode" i18n-aria-label="@@notes.editor.mode">
      <button type="button" role="tab" class="tab" [attr.aria-selected]="!preview()" (click)="preview.set(false)" i18n="@@notes.editor.write">Write</button>
      <button type="button" role="tab" class="tab" [attr.aria-selected]="preview()" (click)="preview.set(true)" data-testid="note-preview-tab" i18n="@@notes.editor.preview">Preview</button>
      <span class="spacer"></span>
      <span class="size" [class.over]="tooLarge()">{{ bytes() }} / {{ maxBytes() }}</span>
    </div>
    @if (preview()) {
      <debrief-markdown-view class="preview" [text]="text()" [placeholder]="emptyPreview" data-testid="note-preview" />
    } @else {
      <textarea
        class="input"
        rows="8"
        aria-label="Note text (Markdown)"
        i18n-aria-label="@@notes.editor.text"
        [attr.placeholder]="placeholder"
        [value]="text()"
        (input)="text.set(value($event))"
        data-testid="note-text"
      ></textarea>
    }
    <div class="row">
      <label class="field grow">
        <span i18n="@@notes.editor.tags">Tags</span>
        <input class="input" type="text" [value]="tags()" (input)="tags.set(value($event))" placeholder="disk, outage" i18n-placeholder="@@notes.editor.tagsPlaceholder" data-testid="note-tags" />
      </label>
      <label class="field">
        <span i18n="@@notes.editor.severity">Severity</span>
        <select class="select" [value]="severity()" (change)="severity.set($any(value($event)))" data-testid="note-severity">
          @for (s of severities; track s) {
            <option [value]="s">{{ severityName(s) }}</option>
          }
        </select>
      </label>
    </div>
    @if (error(); as e) {
      <p class="error" role="alert" data-testid="note-editor-error">{{ e }}</p>
    }
    <div class="foot">
      <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@notes.editor.cancel">Cancel</button>
      <button type="button" class="btn primary" [disabled]="!canSave()" (click)="save()" data-testid="note-save" i18n="@@notes.editor.save">Save note</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; }
    .anchor { margin: 0; color: var(--text-2); overflow-wrap: anywhere; }
    .kind { font-size: var(--fs-xs); text-transform: uppercase; letter-spacing: .04em; color: var(--muted); margin-right: 4px; }
    .tabs { display: flex; align-items: center; gap: 4px; border-bottom: 1px solid var(--border); }
    .tab { border: 0; background: none; padding: 4px 10px; color: var(--text-2); cursor: pointer; border-bottom: 2px solid transparent; }
    .tab[aria-selected='true'] { color: var(--text); border-bottom-color: var(--accent); }
    .spacer { flex: 1; }
    .size { font-size: var(--fs-sm); color: var(--muted); }
    .size.over { color: var(--danger-text); }
    textarea { resize: vertical; min-height: 140px; }
    .preview { min-height: 140px; max-height: 50vh; overflow: auto; padding: 8px; border: 1px solid var(--border); border-radius: 4px; }
    .row { display: flex; gap: 10px; flex-wrap: wrap; }
    .field { display: flex; flex-direction: column; gap: 4px; color: var(--text-2); }
    .grow { flex: 1; min-width: 160px; }
    .error { margin: 0; color: var(--danger-text); }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class NoteEditor implements OnInit {
  protected readonly ref = inject(DIALOG_REF);
  private readonly store = inject(NotesStore);
  /** A new note on this anchor (ignored when `note` is given). */
  readonly anchor = input<NoteAnchor | null>(null);
  /** The note to edit. */
  readonly note = input<NoteItem | null>(null);

  protected readonly text = signal('');
  protected readonly tags = signal('');
  protected readonly severity = signal<NoteSeverity>('info');
  protected readonly preview = signal(false);
  protected readonly saving = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly severities = SEVERITIES;
  protected readonly placeholder = $localize`:@@notes.editor.placeholder:What did you see here? Markdown works: **bold**, lists, \`code\`.`;
  protected readonly emptyPreview = $localize`:@@notes.editor.emptyPreview:Nothing to preview yet.`;

  private readonly target = computed(() => this.note()?.anchor ?? this.anchor());
  protected readonly kind = computed(() => {
    const a = this.target();
    return a ? kindLabel(a.kind) : '';
  });
  protected readonly about = computed(() => {
    const a = this.target();
    return a ? anchorText(a) : '';
  });
  protected readonly maxBytes = computed(() => this.store.maxTextBytes());
  protected readonly bytes = computed(() => encoder.encode(this.text()).length);
  protected readonly tooLarge = computed(() => this.bytes() > this.maxBytes());
  protected readonly canSave = computed(() => !this.saving() && !!this.target() && !!this.text().trim() && !this.tooLarge());

  ngOnInit(): void {
    const n = this.note();
    if (!n) return;
    this.text.set(n.text);
    this.tags.set(n.tags.join(', '));
    this.severity.set(n.severity);
  }

  protected severityName(s: NoteSeverity): string {
    return severityLabel(s);
  }

  protected value(e: Event): string {
    return (e.target as HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement).value;
  }

  async save(): Promise<void> {
    const anchor = this.target();
    if (!this.canSave() || !anchor) return;
    this.saving.set(true);
    this.error.set(null);
    const fields = { text: this.text(), tags: parseTags(this.tags()), severity: this.severity() };
    const n = this.note();
    const r = n ? await this.store.update(n.id, fields) : await this.store.add({ anchor, ...fields });
    this.saving.set(false);
    if (r === null) {
      this.error.set(this.store.actionError());
      this.store.dismissError();
      return;
    }
    this.ref.close(r.id);
  }
}
