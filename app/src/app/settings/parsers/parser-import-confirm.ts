import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';
import { ParserImportCheckDto } from '../../core/api.types';
import { DIALOG_REF } from '../../ui/dialog.service';
import { lineDiff } from './line-diff';

/**
 * Importing a parser whose id is in the library already (ADR 0026 §6): the versions, what replacing changes (a line
 * diff of the two files) and a confirm. Closes with `true` to replace; any other close keeps the library as it is.
 */
@Component({
  selector: 'debrief-parser-import-confirm',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @let c = check();
    <p data-testid="parser-import-question">
      @switch (c.relation) {
        @case ('newer') {
          <ng-container i18n="@@settings.parsers.import.newer">Update <strong>{{ c.name }}</strong> from version {{ existingVersion() }} to {{ c.version }}?</ng-container>
        }
        @case ('same') {
          <ng-container i18n="@@settings.parsers.import.same"><strong>{{ c.name }}</strong> {{ c.version }} is in the library already. Replace it with this file?</ng-container>
        }
        @default {
          <ng-container i18n="@@settings.parsers.import.older">The library has <strong>{{ c.name }}</strong> {{ existingVersion() }}, which is newer than this file's {{ c.version }}. Replace it with the older version?</ng-container>
        }
      }
    </p>
    @if (c.existing?.origin === 'builtin') {
      <p class="note" role="note" i18n="@@settings.parsers.import.overridesBuiltin">{{ c.id }} is built in: the file is saved as an override of it. Removing the override restores the built-in parser.</p>
    }
    <div class="diff mono" role="list" aria-label="Changes to the parser file" i18n-aria-label="@@settings.parsers.import.diffLabel" data-testid="parser-import-diff">
      @for (l of diff(); track $index) {
        <div role="listitem" class="line" [class]="l.kind"><span class="sign" aria-hidden="true">{{ sign(l.kind) }}</span><span class="sr-only">{{ kindLabel(l.kind) }}</span>{{ l.text }}</div>
      } @empty {
        <div role="listitem" class="line same" i18n="@@settings.parsers.import.noChanges">The files are the same.</div>
      }
    </div>
    <div class="foot">
      <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@settings.parsers.import.keep">Keep the library's version</button>
      <button type="button" class="btn primary" [class.danger]="c.relation === 'older'" data-testid="parser-import-replace" (click)="ref.close(true)">{{ confirmLabel() }}</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; }
    p { margin: 0; color: var(--text-2); }
    .note { padding: 6px 8px; border-inline-start: 2px solid var(--accent); color: var(--text-2); }
    .diff { max-height: 320px; overflow: auto; border: 1px solid var(--border); border-radius: 6px; background: var(--code-bg); font-size: var(--fs-sm); padding: 4px 0; }
    .line { white-space: pre-wrap; overflow-wrap: anywhere; padding: 0 8px; }
    .line.add { background: color-mix(in srgb, var(--ok) 18%, transparent); }
    .line.del { background: color-mix(in srgb, var(--lvl-error) 18%, transparent); }
    .sign { display: inline-block; width: 1.5ch; color: var(--muted); }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
    .btn.primary.danger { background: var(--lvl-error); border-color: var(--lvl-error); color: var(--accent-ink); }
  `,
})
export class ParserImportConfirm {
  protected readonly ref = inject(DIALOG_REF);
  readonly check = input.required<ParserImportCheckDto>();

  protected readonly existingVersion = computed(() => this.check().existing?.version ?? '');
  /** The library's file against the imported one (only changed lines and their neighbourhood would be noise-free, but parser files are short). */
  protected readonly diff = computed(() => {
    const c = this.check();
    const lines = lineDiff(c.existing?.toml ?? '', c.toml);
    return lines.every((l) => l.kind === 'same') ? [] : lines;
  });
  protected readonly confirmLabel = computed(() =>
    this.check().relation === 'newer' ? $localize`:@@settings.parsers.import.update:Update` : $localize`:@@settings.parsers.import.replace:Replace`,
  );

  protected sign(kind: 'same' | 'add' | 'del'): string {
    return kind === 'add' ? '+' : kind === 'del' ? '−' : ' ';
  }

  protected kindLabel(kind: 'same' | 'add' | 'del'): string {
    return kind === 'add' ? $localize`:@@settings.parsers.import.added:Added: ` : kind === 'del' ? $localize`:@@settings.parsers.import.removed:Removed: ` : '';
  }
}
