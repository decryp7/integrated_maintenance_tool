import { ChangeDetectionStrategy, Component, computed, effect, inject, input, OnInit, signal, untracked } from '@angular/core';
import { ParserDocDto, ParsePreview, SourceInfo } from '../../core/api.types';
import { ImtBackend } from '../../core/backend';
import { CmdError } from '../../core/cmd-error';
import { formatFull } from '../../core/format';
import { describeError } from '../../core/logger';
import { WorkspaceStore } from '../../core/workspace.store';
import { TOML_DOCUMENT, TomlEditor } from '../../profile/toml-editor';
import { ConfirmDialog } from '../../ui/confirm-dialog';
import { DIALOG_REF, DialogService } from '../../ui/dialog.service';
import { SettingsStore } from '../settings.store';
import { ParserDraft } from './parser-draft';
import { ParsersBackend } from './parsers-backend';

/** How much of the chosen file the preview reads, and how many events it shows. */
const HEAD_BYTES = 32 * 1024;
const PREVIEW_LIMIT = 50;
const PREVIEW_DEBOUNCE_MS = 300;

/**
 * Edits one parser's TOML (ADR 0026): the engine validates it as you type (errors pinned to line and column) and
 * previews it against a file of this window. Saving a built-in saves an override. Closes with the library after a
 * save, or without a result.
 */
@Component({
  selector: 'imt-parser-editor',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [TomlEditor],
  providers: [ParserDraft, { provide: TOML_DOCUMENT, useExisting: ParserDraft }],
  template: `
    @if (doc().origin === 'builtin') {
      <p class="note" role="note" data-testid="parser-builtin-note" i18n="@@settings.parsers.editor.builtinNote">This parser is built in. Saving keeps your version as an override of it; removing the override restores the built-in parser.</p>
    } @else if (doc().origin === 'override') {
      <p class="note" role="note" i18n="@@settings.parsers.editor.overrideNote">You are editing your override of a built-in parser.</p>
    }
    <div class="toml">
      <imt-toml-editor label="Parser TOML" i18n-label="@@settings.parsers.editor.tomlLabel" />
    </div>
    <div class="status" role="status" data-testid="parser-validation">
      @if (draft.validating()) {
        <span class="muted" i18n="@@settings.parsers.editor.checking">Checking…</span>
      } @else if (draft.validation()?.ok) {
        <span class="ok" i18n="@@settings.parsers.editor.valid">Valid: {{ draft.validation()!.name }} {{ draft.validation()!.version }} ({{ draft.validation()!.id }})</span>
      }
    </div>

    <section class="preview" aria-label="Preview" i18n-aria-label="@@settings.parsers.editor.previewLabel">
      @if (sources().length) {
        <label class="lbl"><ng-container i18n="@@settings.parsers.editor.previewFile">Preview with</ng-container>
          <select class="select" data-testid="parser-preview-file" [value]="sourceId() ?? ''" (change)="pickSource($any($event.target).value)">
            @for (s of sources(); track s.id) { <option [value]="s.id" [selected]="s.id === sourceId()">{{ s.name }}</option> }
          </select>
        </label>
        @if (headError(); as e) {
          <div class="err" role="alert">{{ e }} <button type="button" class="btn" (click)="loadHead()" i18n="@@settings.parsers.editor.retry">Retry</button></div>
        }
        <div class="status" data-testid="parser-preview-summary">
          @if (previewError() ?? preview()?.error; as pe) { <span class="err">{{ pe }}</span> }
          @else if (!preview()) { <span class="muted" i18n="@@settings.parsers.editor.previewing">Reading the start of the file…</span> }
          @else { <ng-container i18n="@@settings.parsers.editor.previewSummary"><strong>{{ preview()!.total }}</strong> events read from the start of the file, {{ preview()!.skipped }} lines skipped.</ng-container> }
        </div>
        @if (preview(); as p) {
          <div class="rows" role="table" aria-label="Parse preview" i18n-aria-label="@@settings.parsers.editor.previewTable">
            <div class="prow head" role="row"><span role="columnheader" i18n="@@settings.parsers.editor.col.time">Time</span><span role="columnheader" i18n="Log severity level|Column header@@settings.parsers.editor.col.level">Level</span><span role="columnheader" i18n="@@settings.parsers.editor.col.message">Message</span></div>
            @for (r of p.records; track r.line) {
              <div class="prow" role="row"><span role="cell" class="mono">{{ time(r.ts) }}</span><span role="cell" class="mono">{{ r.level }}</span><span role="cell">{{ r.message }}</span></div>
            }
          </div>
        }
      } @else {
        <p class="muted" i18n="@@settings.parsers.editor.noFiles">Open a log in this window to preview the parser against it.</p>
      }
    </section>

    @if (error(); as e) { <div class="err" role="alert">{{ e }}</div> }
    <div class="foot">
      <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@settings.parsers.editor.cancel">Cancel</button>
      <button type="button" class="btn primary" data-testid="parser-save" [disabled]="!canSave() || saving()" (click)="save()">{{ saveLabel() }}</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 8px; height: min(720px, calc(100vh - 140px)); min-height: 0; }
    .note { margin: 0; padding: 6px 8px; border-inline-start: 2px solid var(--accent); color: var(--text-2); }
    .toml { flex: 1 1 280px; min-height: 160px; display: flex; flex-direction: column; border: 1px solid var(--border); border-radius: 6px; overflow: hidden; }
    .status { font-size: var(--fs-sm); color: var(--text-2); min-height: 1.4em; }
    .muted { color: var(--muted); }
    .ok { color: var(--ok); }
    .err { color: var(--danger-text); }
    .preview { flex: 0 1 auto; display: flex; flex-direction: column; gap: 6px; min-height: 0; }
    .lbl { display: flex; align-items: center; gap: 8px; color: var(--text-2); }
    .rows { max-height: 180px; overflow: auto; border: 1px solid var(--border); border-radius: 6px; font-size: var(--fs-sm); }
    .prow { display: grid; grid-template-columns: 200px 90px minmax(0, 1fr); gap: 8px; padding: 3px 8px; border-bottom: 1px solid var(--border); }
    .prow.head { position: sticky; top: 0; background: var(--bg-3); color: var(--muted); font-weight: 600; }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class ParserEditor implements OnInit {
  protected readonly ref = inject(DIALOG_REF);
  protected readonly draft = inject(ParserDraft);
  private readonly parsers = inject(ParsersBackend);
  private readonly backend = inject(ImtBackend);
  private readonly workspace = inject(WorkspaceStore);
  private readonly settings = inject(SettingsStore);
  private readonly dialogs = inject(DialogService);

  readonly doc = input.required<ParserDocDto>();
  /** A parser that is not in the library yet ("New blank"): no previous id. */
  readonly isNew = input(false);
  /** Sources this parser read: the preview starts with one of them. */
  readonly readSources = input<number[]>([]);

  protected readonly sources = computed<SourceInfo[]>(() => this.workspace.summary()?.sources ?? []);
  protected readonly sourceId = signal<number | null>(null);
  private readonly head = signal('');
  protected readonly headError = signal<string | null>(null);
  protected readonly preview = signal<ParsePreview | null>(null);
  protected readonly previewError = signal<string | null>(null);
  protected readonly saving = signal(false);
  protected readonly error = signal<string | null>(null);
  private previewSeq = 0;

  protected readonly canSave = computed(() => this.draft.valid() && (this.draft.dirty() || this.isNew()));
  protected readonly saveLabel = computed(() =>
    this.saving()
      ? $localize`:@@settings.parsers.editor.saving:Saving…`
      : this.doc().origin === 'builtin'
        ? $localize`:@@settings.parsers.editor.saveOverride:Save as override`
        : $localize`:@@settings.parsers.editor.save:Save`,
  );

  constructor() {
    // The engine previews the edited TOML against the chosen file (after a short pause while typing).
    effect((onCleanup) => {
      const toml = this.draft.text();
      const head = this.head();
      if (!head) return;
      const timer = setTimeout(() => untracked(() => void this.runPreview(toml, head)), PREVIEW_DEBOUNCE_MS);
      onCleanup(() => clearTimeout(timer));
    });
  }

  ngOnInit(): void {
    this.draft.open(this.doc().toml);
    const sources = this.sources();
    const preferred = sources.find((s) => this.readSources().includes(s.id)) ?? sources[0];
    if (preferred) {
      this.sourceId.set(preferred.id);
      void this.loadHead();
    }
  }

  protected pickSource(id: string): void {
    this.sourceId.set(Number(id));
    void this.loadHead();
  }

  protected async loadHead(): Promise<void> {
    const source = this.sources().find((s) => s.id === this.sourceId());
    if (!source) return;
    this.headError.set(null);
    this.head.set('');
    this.preview.set(null);
    try {
      this.head.set(await this.backend.readFileHead(source.path, HEAD_BYTES));
    } catch (e) {
      this.headError.set(describeError(e));
    }
  }

  private async runPreview(toml: string, sample: string): Promise<void> {
    const mine = ++this.previewSeq;
    const fileName = this.sources().find((s) => s.id === this.sourceId())?.name ?? '';
    this.previewError.set(null);
    try {
      const result = await this.backend.previewParse({ toml, sample, fileName, limit: PREVIEW_LIMIT });
      if (mine === this.previewSeq) this.preview.set(result);
    } catch (e) {
      if (mine === this.previewSeq) this.previewError.set(describeError(e));
    }
  }

  protected time(ts: number | null): string {
    return ts === null ? '–' : formatFull(ts, this.workspace.timezone(), this.settings.dateFormat());
  }

  protected async save(): Promise<void> {
    if (!this.canSave()) return;
    this.saving.set(true);
    this.error.set(null);
    const toml = this.draft.text();
    const previousId = this.isNew() ? null : this.doc().id;
    try {
      let library;
      try {
        library = await this.parsers.save(toml, { previousId });
      } catch (e) {
        if (!(e instanceof CmdError && e.code === 'fileExists') || !(await this.confirmReplace(e.message))) throw e;
        library = await this.parsers.save(toml, { previousId, overwrite: true });
      }
      this.ref.close(library);
    } catch (e) {
      this.error.set(describeError(e));
    } finally {
      this.saving.set(false);
    }
  }

  private async confirmReplace(message: string): Promise<boolean> {
    const ref = this.dialogs.open<boolean>(ConfirmDialog, {
      title: $localize`:@@settings.parsers.editor.replaceTitle:Replace parser?`,
      width: 460,
      inputs: { message, detail: $localize`:@@settings.parsers.editor.replaceDetail:Replacing it overwrites that parser file.`, confirmLabel: $localize`:@@settings.parsers.editor.replace:Replace`, danger: true },
    });
    return (await ref.closed) === true;
  }
}
