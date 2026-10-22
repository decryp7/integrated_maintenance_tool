import { ChangeDetectionStrategy, Component, computed, inject, input, OnInit, signal } from '@angular/core';
import { EncodingOption, SourceInfo } from '../../core/api.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { errorMessage } from '../../tools/tool-errors';
import { DIALOG_REF } from '../../ui/dialog.service';
import { EncodingBackend } from './encoding-backend';
import { AUTO_ENCODING, encodingText, initialEncodingChoice } from './encoding-format';

/**
 * Explorer ▸ "Encoding…": reads one source again with an encoding the user chooses, or detects it again. The encoding
 * the user chose before is selected at first, else "Automatic".
 */
@Component({
  selector: 'imt-encoding-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <p class="intro" i18n="@@sources.encoding.dialog.intro">The file is read again with the encoding you choose; its events are replaced.</p>
    @if (current(); as c) {
      <p class="now" data-testid="encoding-current" i18n="@@sources.encoding.dialog.current">Now: {{ c }}</p>
    }
    <fieldset>
      <legend class="sr-only" i18n="@@sources.encoding.dialog.legend">Encoding for {{ source().name }}</legend>
      <label class="choice auto">
        <input type="radio" name="imt-encoding" [value]="auto" [checked]="choice() === auto" (change)="pick(auto)" />
        <span class="name" i18n="@@sources.encoding.dialog.automatic">Automatic (detect)</span>
        <span class="hint" i18n="@@sources.encoding.dialog.automaticHint">IMT guesses it from the file's bytes</span>
      </label>
      <div class="list" role="presentation">
        @for (o of options(); track o.label) {
          <label class="choice" [attr.data-encoding]="o.label">
            <input type="radio" name="imt-encoding" [value]="o.label" [checked]="choice() === o.label" (change)="pick(o.label)" />
            <span class="name">{{ o.label }}</span>
            <span class="hint">{{ o.name }}</span>
          </label>
        }
      </div>
    </fieldset>
    @if (error(); as e) { <p class="err" role="alert">{{ e }}</p> }
    <div class="foot">
      <button type="button" class="btn ghost" data-testid="encoding-cancel" (click)="ref.close()" i18n="@@sources.encoding.dialog.cancel">Cancel</button>
      <button type="button" class="btn primary" data-testid="encoding-apply" [disabled]="busy()" (click)="apply()" i18n="@@sources.encoding.dialog.apply">Apply</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; }
    p { margin: 0; }
    .now, .hint { color: var(--muted); font-size: var(--fs-xs); }
    fieldset { border: 0; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; min-width: 0; }
    .list { max-height: 320px; overflow: auto; display: flex; flex-direction: column; gap: 2px; border: 1px solid var(--border); border-radius: 6px; padding: 4px; }
    .choice { display: flex; align-items: center; gap: 8px; padding: 4px 6px; border-radius: 5px; cursor: pointer; }
    .choice:hover { background: var(--bg-hover); }
    .name { font-weight: 500; }
    .err { color: var(--danger-text); }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class EncodingDialog implements OnInit {
  protected readonly ref = inject(DIALOG_REF);
  private readonly backend = inject(EncodingBackend);
  private readonly workspace = inject(WorkspaceStore);
  readonly source = input.required<SourceInfo>();
  protected readonly auto = AUTO_ENCODING;
  protected readonly options = signal<EncodingOption[]>([]);
  protected readonly error = signal<string | null>(null);
  protected readonly busy = signal(false);
  /** What the user picked; until they pick, the source's own choice (see `initialEncodingChoice`). */
  private readonly picked = signal<string | null>(null);
  protected readonly choice = computed(() => this.picked() ?? initialEncodingChoice(this.source().encoding));
  protected readonly current = computed(() => {
    const info = this.source().encoding;
    return info ? encodingText(info) : null;
  });

  ngOnInit(): void {
    this.backend.options().then(
      (o) => this.options.set(o),
      (e: unknown) => this.error.set(errorMessage(e)),
    );
  }

  protected pick(label: string): void {
    this.picked.set(label);
  }

  /** Starts the re-read, closes, and lets the workspace follow the job like an import. */
  protected async apply(): Promise<void> {
    const label = this.choice();
    this.busy.set(true);
    this.error.set(null);
    try {
      const job = await this.backend.setSourceEncoding(this.source().id, label === AUTO_ENCODING ? null : label);
      this.ref.close(true);
      await this.workspace.followReread(job);
    } catch (e) {
      this.error.set(errorMessage(e));
      this.busy.set(false);
    }
  }
}
