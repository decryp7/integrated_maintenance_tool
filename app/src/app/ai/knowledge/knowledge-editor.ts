import { ChangeDetectionStrategy, Component, computed, inject, input, OnInit, signal } from '@angular/core';
import { MarkdownView } from '../../share/report/markdown-view';
import { DIALOG_REF } from '../../ui/dialog.service';
import { formatBytes } from '../ai-format';
import { KnowledgeInfo, KnowledgeScope } from '../ai.types';
import { KnowledgeStore } from './knowledge.store';

const encoder = new TextEncoder();

/**
 * Writes or edits a knowledge file: its name and Markdown text, with a preview (rendered as text nodes, never HTML).
 * Saving goes through the engine, which checks the name and size; the dialog stays open with the reason when it
 * refuses. Closes with `true` once saved.
 */
@Component({
  selector: 'imt-knowledge-editor',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [MarkdownView],
  template: `
    <label class="field">
      <span i18n="@@ai.knowledge.editor.name">Name</span>
      <input class="input" type="text" maxlength="80" [value]="name()" (input)="name.set(value($event))" data-testid="knowledge-name" />
    </label>
    <div class="tabs" role="tablist" aria-label="Editor mode" i18n-aria-label="@@ai.knowledge.editor.mode">
      <button type="button" role="tab" class="tab" [attr.aria-selected]="!preview()" (click)="preview.set(false)" i18n="@@ai.knowledge.editor.write">Write</button>
      <button type="button" role="tab" class="tab" [attr.aria-selected]="preview()" (click)="preview.set(true)" data-testid="knowledge-preview-tab" i18n="@@ai.knowledge.editor.preview">Preview</button>
      <span class="spacer"></span>
      <span class="size" [class.over]="tooLarge()" data-testid="knowledge-size">{{ sizeLine() }}</span>
    </div>
    @if (preview()) {
      <imt-markdown-view class="preview" [text]="text()" [placeholder]="emptyPreview" data-testid="knowledge-preview" />
    } @else {
      <textarea
        class="input mono"
        rows="16"
        spellcheck="false"
        aria-label="Markdown text"
        i18n-aria-label="@@ai.knowledge.editor.text"
        [attr.placeholder]="placeholder"
        [value]="text()"
        (input)="text.set(value($event))"
        data-testid="knowledge-text"
      ></textarea>
    }
    @if (loadError(); as e) {
      <p class="error" role="alert">{{ e }}</p>
    }
    @if (error(); as e) {
      <p class="error" role="alert" data-testid="knowledge-editor-error">{{ e }}</p>
    }
    <div class="foot">
      <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@ai.knowledge.editor.cancel">Cancel</button>
      <button type="button" class="btn primary" [disabled]="!canSave()" (click)="save()" data-testid="knowledge-save" i18n="@@ai.knowledge.editor.save">Save</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; }
    .field { display: flex; flex-direction: column; gap: 4px; color: var(--text-2); }
    .tabs { display: flex; align-items: center; gap: 4px; border-bottom: 1px solid var(--border); }
    .tab { border: 0; background: none; padding: 4px 10px; color: var(--text-2); cursor: pointer; border-bottom: 2px solid transparent; }
    .tab[aria-selected='true'] { color: var(--text); border-bottom-color: var(--accent); }
    .spacer { flex: 1; }
    .size { font-size: var(--fs-sm); color: var(--muted); }
    .size.over { color: var(--danger-text); }
    textarea { resize: vertical; min-height: 240px; }
    .preview { min-height: 240px; max-height: 50vh; overflow: auto; padding: 8px; border: 1px solid var(--border); border-radius: 4px; }
    .error { margin: 0; color: var(--danger-text); }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class KnowledgeEditor implements OnInit {
  protected readonly ref = inject(DIALOG_REF);
  private readonly store = inject(KnowledgeStore);
  /** The file to edit; null writes a new one in `scope`. */
  readonly file = input<KnowledgeInfo | null>(null);
  readonly scope = input<KnowledgeScope>('global');
  readonly maxBytes = input(64 * 1024);

  protected readonly name = signal('');
  protected readonly text = signal('');
  protected readonly preview = signal(false);
  protected readonly saving = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly loadError = signal<string | null>(null);
  private readonly loaded = signal(true);

  protected readonly placeholder = $localize`:@@ai.knowledge.editor.placeholder:# Glossary

- **ACME-GW**: the payment gateway (logs: gateway-*.log)
- **E1042**: card declined by the bank; no action needed`;
  protected readonly emptyPreview = $localize`:@@ai.knowledge.editor.emptyPreview:Nothing to preview yet.`;

  private readonly bytes = computed(() => encoder.encode(this.text()).length);
  protected readonly tooLarge = computed(() => this.bytes() > this.maxBytes());
  protected readonly sizeLine = computed(() => $localize`:@@ai.knowledge.editor.size:${formatBytes(this.bytes())}:size: of ${formatBytes(this.maxBytes())}:max:`);
  protected readonly canSave = computed(() => this.loaded() && !this.saving() && !!this.name().trim() && !this.tooLarge());

  async ngOnInit(): Promise<void> {
    const f = this.file();
    if (!f) return;
    this.name.set(f.name);
    this.loaded.set(false);
    try {
      this.text.set(await this.store.text(f));
      this.loaded.set(true);
    } catch (e) {
      this.loadError.set((e as Error).message);
    }
  }

  protected value(e: Event): string {
    return (e.target as HTMLInputElement | HTMLTextAreaElement).value;
  }

  async save(): Promise<void> {
    if (!this.canSave()) return;
    this.saving.set(true);
    this.error.set(null);
    const f = this.file();
    const r = f ? await this.store.update(f, { name: this.name(), text: this.text() }) : await this.store.create(this.scope(), this.name(), this.text());
    this.saving.set(false);
    if (r === null) {
      this.error.set(this.store.actionError());
      this.store.dismissError();
      return;
    }
    this.ref.close(true);
  }
}
