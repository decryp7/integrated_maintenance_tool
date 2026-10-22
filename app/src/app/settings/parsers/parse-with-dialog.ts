import { ChangeDetectionStrategy, Component, computed, inject, input, OnInit, signal } from '@angular/core';
import { ParserLibraryDto, SourceInfo } from '../../core/api.types';
import { DIALOG_REF } from '../../ui/dialog.service';
import { ParsersStore } from './parsers.store';

/** The "Automatic" choice (the engine chooses the parser again). */
const AUTO = '';

/**
 * What the dialog selects at first: the parser the user chose for the source before, else "Automatic" (also when
 * a parser was matched automatically: confirming without a change keeps the automatic choice instead of pinning
 * the parser that happens to read it now).
 */
export function initialChoice(library: ParserLibraryDto | null, sourceId: number): string {
  const chosen = library?.parsers.find((p) => p.sources.some((s) => s.id === sourceId && s.chosen));
  return chosen?.id ?? AUTO;
}

/**
 * Explorer ▸ "Parse with…" (ADR 0026 §3): reads one source again with a parser of the library, enabled or not, or
 * with the automatic choice. The parser the user chose for it before is selected at first, else "Automatic".
 */
@Component({
  selector: 'imt-parse-with-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <fieldset>
      <legend class="sr-only" i18n="@@settings.parsers.parseWith.legend">Parser for {{ source().name }}</legend>
      <label class="choice auto">
        <input type="radio" name="imt-parse-with" [value]="auto" [checked]="choice() === auto" (change)="pick(auto)" />
        <span class="name" i18n="@@settings.parsers.parseWith.automatic">Automatic</span>
        <span class="hint" i18n="@@settings.parsers.parseWith.automaticHint">the enabled parser that reads the file best</span>
      </label>
      <div class="list" role="presentation">
        @for (p of store.choices(); track p.id) {
          <label class="choice" [class.off]="!p.enabled" [attr.data-parser]="p.id">
            <input type="radio" name="imt-parse-with" [value]="p.id" [checked]="choice() === p.id" (change)="pick(p.id)" />
            <span class="name">{{ p.name }}</span>
            <span class="mono hint">{{ p.id }}</span>
            @if (!p.enabled) { <span class="badge" i18n="Parser state badge@@settings.parsers.parseWith.disabled">disabled</span> }
            @if (p.id === source().parser) { <span class="badge current" i18n="@@settings.parsers.parseWith.current">read with it now</span> }
          </label>
        }
      </div>
    </fieldset>
    @if (store.loadError(); as e) { <p class="err" role="alert">{{ e }}</p> }
    <div class="foot">
      <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@settings.parsers.parseWith.cancel">Cancel</button>
      <button type="button" class="btn primary" data-testid="parse-with-apply" [disabled]="!store.library()" (click)="apply()" i18n="@@settings.parsers.parseWith.apply">Read again</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; }
    fieldset { border: 0; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; min-width: 0; }
    .list { max-height: 320px; overflow: auto; display: flex; flex-direction: column; gap: 2px; border: 1px solid var(--border); border-radius: 6px; padding: 4px; }
    .choice { display: flex; align-items: center; gap: 8px; padding: 4px 6px; border-radius: 5px; cursor: pointer; }
    .choice:hover { background: var(--bg-hover); }
    .choice.off .name { color: var(--muted); }
    .name { font-weight: 500; }
    .hint { color: var(--muted); font-size: var(--fs-xs); }
    .badge { font-size: var(--fs-2xs); padding: 0 6px; border-radius: 8px; border: 1px solid var(--border-3); color: var(--text-2); }
    .badge.current { border-color: var(--accent); }
    .err { margin: 0; color: var(--danger-text); }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class ParseWithDialog implements OnInit {
  protected readonly ref = inject(DIALOG_REF);
  protected readonly store = inject(ParsersStore);
  readonly source = input.required<SourceInfo>();
  protected readonly auto = AUTO;
  /** What the user picked; until they pick, the source's own choice (see `initialChoice`). */
  private readonly picked = signal<string | null>(null);
  protected readonly choice = computed(() => this.picked() ?? initialChoice(this.store.library(), this.source().id));

  ngOnInit(): void {
    void this.store.load();
  }

  protected pick(id: string): void {
    this.picked.set(id);
  }

  protected apply(): void {
    const id = this.choice();
    void this.store.assign(this.source().id, id === AUTO ? null : id);
    this.ref.close(true);
  }
}
