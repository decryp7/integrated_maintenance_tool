import { computed, inject, Injectable, OnDestroy, signal } from '@angular/core';
import { ParserValidationDto } from '../../core/api.types';
import { describeError } from '../../core/logger';
import { TomlDocument } from '../../profile/toml-editor';
import { ParsersBackend } from './parsers-backend';

const VALIDATE_DEBOUNCE_MS = 250;

/**
 * The parser TOML being edited (one per editor): the text, and the engine's validation of it (debounced while
 * typing, with line and column). The text is the source of truth; nothing is saved until the editor saves.
 */
@Injectable()
export class ParserDraft implements TomlDocument, OnDestroy {
  private readonly backend = inject(ParsersBackend);

  readonly text = signal('');
  /** The text it was opened with (dirty = changed since). */
  readonly original = signal('');
  readonly validation = signal<ParserValidationDto | null>(null);
  /** Validation itself failed (the engine was not reachable). */
  readonly validationError = signal<string | null>(null);
  readonly validating = signal(false);
  readonly dirty = computed(() => this.text() !== this.original());
  readonly problem = computed(() => {
    const failed = this.validationError();
    if (failed) return { message: failed, line: null, column: null };
    const v = this.validation();
    return v && !v.ok && v.error ? { message: v.error.message, line: v.error.line, column: v.error.column } : null;
  });
  /** Saving makes sense: validated, valid, and the validation is of the current text. */
  readonly valid = computed(() => !!this.validation()?.ok && !this.validating());

  private timer: ReturnType<typeof setTimeout> | undefined;
  private seq = 0;

  /** Starts editing `toml` (validated at once). */
  open(toml: string): void {
    this.original.set(toml);
    this.text.set(toml);
    void this.validate();
  }

  setText(text: string): void {
    this.text.set(text);
    this.validating.set(true);
    clearTimeout(this.timer);
    this.timer = setTimeout(() => void this.validate(), VALIDATE_DEBOUNCE_MS);
  }

  /** Asks the engine about the current text; a newer request wins. */
  async validate(): Promise<void> {
    clearTimeout(this.timer);
    const mine = ++this.seq;
    this.validating.set(true);
    try {
      const v = await this.backend.validate(this.text());
      if (mine !== this.seq) return;
      this.validation.set(v);
      this.validationError.set(null);
    } catch (e) {
      if (mine !== this.seq) return;
      this.validation.set(null);
      this.validationError.set(describeError(e));
    } finally {
      if (mine === this.seq) this.validating.set(false);
    }
  }

  ngOnDestroy(): void {
    clearTimeout(this.timer);
  }
}
