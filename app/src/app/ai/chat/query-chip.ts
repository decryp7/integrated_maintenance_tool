import { ChangeDetectionStrategy, Component, computed, input, output, signal } from '@angular/core';

/**
 * The exact query the assistant ran, shown under its tool chip so the user can see, reuse and correct it. A
 * query-language query has "Open in Events" and "Edit" (edit in place, then open the edited text); SQL, which the
 * assistant uses only as a fallback, is shown read-only with an SQL tag. Presentation only: opening goes through
 * the parent (the citation actions), which asks the engine to run the text.
 */
@Component({
  selector: 'debrief-query-chip',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="qchip" [attr.data-language]="language()" data-testid="ai-query-chip">
      @if (editing()) {
        <input
          class="edit"
          type="text"
          spellcheck="false"
          [value]="draft()"
          (input)="draft.set(asValue($event))"
          (keydown.enter)="openEdited()"
          (keydown.escape)="editing.set(false)"
          aria-label="Edit the query"
          i18n-aria-label="@@ai.queryChip.editLabel"
          data-testid="ai-query-edit"
        />
        <button type="button" class="act" (click)="openEdited()" data-testid="ai-query-open-edited" i18n="@@ai.queryChip.openEdited">Open in Events</button>
        <button type="button" class="act" (click)="editing.set(false)" data-testid="ai-query-cancel" i18n="@@ai.queryChip.cancel">Cancel</button>
      } @else {
        @if (language() === 'sql') {
          <span class="tag" data-testid="ai-query-sql-tag" i18n="@@ai.queryChip.sqlTag">SQL</span>
        }
        <code class="text debrief-selectable" data-testid="ai-query-text">{{ text() }}</code>
        @if (language() === 'ql') {
          <button type="button" class="act" (click)="open.emit(text())" data-testid="ai-query-open" i18n="@@ai.queryChip.open">Open in Events</button>
          <button type="button" class="act" (click)="startEdit()" data-testid="ai-query-edit-start" i18n="@@ai.queryChip.edit">Edit</button>
        }
      }
    </div>
  `,
  styles: `
    :host { display: block; }
    .qchip { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; padding: 4px 8px; border: 1px solid var(--border); border-radius: 6px; background: var(--bg-1); font-size: var(--fs-xs); }
    .text { flex: 1 1 12rem; min-width: 0; overflow-wrap: anywhere; font-family: var(--font-mono, monospace); }
    .edit { flex: 1 1 12rem; min-width: 0; font: inherit; font-family: var(--font-mono, monospace); color: var(--fg-0); background: var(--bg-0); border: 1px solid var(--border); border-radius: 4px; padding: 2px 6px; }
    .edit:focus-visible, .act:focus-visible { outline: 2px solid var(--focus-ring); }
    .tag { font-weight: 600; color: var(--accent-text); }
    .act { all: unset; cursor: pointer; color: var(--accent-text); text-decoration: underline; }
  `,
})
export class QueryChip {
  /** The query text as the assistant ran it. */
  readonly query = input.required<string>();
  /** `ql` (the query language: editable, opens in Events) or `sql` (the assistant's fallback: read-only). */
  readonly language = input<'ql' | 'sql'>('ql');
  /** The text to run in Events (the original, or what the user edited it to). */
  readonly open = output<string>();

  protected readonly editing = signal(false);
  protected readonly draft = signal('');
  private readonly edited = signal<string | null>(null);
  protected readonly text = computed(() => this.edited() ?? this.query());

  protected startEdit(): void {
    this.draft.set(this.text());
    this.editing.set(true);
  }

  protected openEdited(): void {
    const q = this.draft().trim();
    if (!q) return;
    this.edited.set(q);
    this.editing.set(false);
    this.open.emit(q);
  }

  protected asValue(e: Event): string {
    return (e.target as HTMLInputElement).value;
  }
}
