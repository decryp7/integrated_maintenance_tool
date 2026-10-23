import { ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, input, OnInit, signal, untracked, viewChild } from '@angular/core';
import { AiButton } from '../ai/ai-button';
import { AiLauncher } from '../ai/ai-launcher';
import { selectionWithin } from '../context/selection';
import { ParserDocDto, ParserDraftRequest, ParsePreview } from '../core/api.types';
import { DebriefBackend } from '../core/backend';
import { formatFull } from '../core/format';
import { describeError } from '../core/logger';
import { WorkspaceStore } from '../core/workspace.store';
import { SettingsStore } from '../settings/settings.store';
import { ParsersBackend } from '../settings/parsers/parsers-backend';
import { ParsersStore } from '../settings/parsers/parsers.store';
import { HelpHint } from '../ui/help-hint';
import { DIALOG_REF } from '../ui/dialog.service';
import { DIALOG_FORM_STYLES } from '../context/dialog-styles';
import { FormatPart, Marks, regexFromMarks } from './format-parse';

/** Sample lines offered for marking, and records the engine previews. */
const PREVIEW_LINES = 50;
/** How much of the file the engine is shown. */
const HEAD_BYTES = 32 * 1024;
const PREVIEW_DEBOUNCE_MS = 250;
const PARTS: { id: FormatPart; label: string }[] = [
  { id: 'timestamp', label: $localize`:@@format.wizard.part.timestamp:Timestamp` },
  { id: 'level', label: $localize`:Log severity level|Part of a log line@@format.wizard.part.level:Level` },
  { id: 'message', label: $localize`:@@format.wizard.part.message:Message` },
];

/**
 * "Set up format": teaches Debrief how to read a file, from marks on a sample line or a Serilog template (a new parser,
 * ADR 0026), or how its parser reads the timestamps (a changed copy of that parser). The engine composes the
 * candidate parser (`parser_draft`) and previews it; Save adds it to the library and reads the files again.
 */
@Component({
  selector: 'debrief-format-wizard',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [HelpHint, AiButton],
  host: { '(document:selectionchange)': 'onSelection()' },
  template: `
    @if (mode() === 'timestamps') {
      <p i18n="@@format.wizard.timestampsIntro">Debrief read the lines of <span class="mono">{{ name() }}</span> but not their timestamps, so those events are missing from the timeline. Tell it how the timestamps are written.</p>
    } @else if (baseParser()) {
      <p i18n="@@format.wizard.newParserIntro">Show Debrief how a log line of <span class="mono">{{ name() }}</span> is built: it saves a new parser for this kind of file (now read with {{ baseParser() }}).</p>
    } @else {
    <p><ng-container i18n="@@format.wizard.intro">Debrief did not recognise <span class="mono">{{ name() }}</span>. Show it how a log line is built and it will read this kind of file from now on.</ng-container>&ngsp;<debrief-help label="Format wizard" i18n-label="@@format.wizard.helpLabel" i18n-text="@@format.wizard.helpText" text="Select the timestamp, then the level, then the message on a sample line and press the matching button. Debrief builds a regular expression from your marks and shows how the first lines parse." /></p>
    }

    @if (loadError(); as e) {
      <div class="err" role="alert">{{ e }} <button type="button" class="btn" (click)="loadHead()" i18n="@@format.wizard.retry">Retry</button></div>
    }

    <div class="seg" role="group" aria-label="How to describe the format" i18n-aria-label="@@format.wizard.modes">
      <button type="button" class="btn" [class.active]="mode() === 'marks'" [attr.aria-pressed]="mode() === 'marks'" (click)="mode.set('marks')" i18n="@@format.wizard.modeMarks">Mark on a sample line</button>
      <button type="button" class="btn" [class.active]="mode() === 'template'" [attr.aria-pressed]="mode() === 'template'" (click)="mode.set('template')" i18n="@@format.wizard.modeTemplate">Serilog output template</button>
      @if (timestampFormat() !== undefined) {
        <button type="button" class="btn" [class.active]="mode() === 'timestamps'" [attr.aria-pressed]="mode() === 'timestamps'" (click)="mode.set('timestamps')" i18n="@@format.wizard.modeTimestamps">Timestamp format</button>
      }
    </div>

    @if (mode() === 'marks') {
      @if (lines().length) {
        <label class="lbl"><ng-container i18n="@@format.wizard.sampleLine">Sample line</ng-container>
          <select class="select mono" [value]="sampleIndex()" (change)="pickSample($any($event.target).value)">
            @for (l of lines(); track $index) { <option [value]="$index" [selected]="$index === sampleIndex()">{{ $index + 1 }}: {{ l.slice(0, 80) }}</option> }
          </select>
        </label>
        <div class="status" i18n="@@format.wizard.selectHint">Select part of the line with the mouse or Shift+arrows, then press a button:</div>
        <div #sample class="box" tabindex="0" role="textbox" aria-readonly="true" aria-label="Sample line" i18n-aria-label="@@format.wizard.sampleLineLabel" data-testid="sample-line">
          @for (seg of segments(); track $index) {
            @if (seg.part) {
              <span class="hl" [attr.data-part]="seg.part">{{ seg.text }}</span>
            } @else {
              <span>{{ seg.text }}</span>
            }
          }
        </div>
        <div class="row">
          @for (p of parts; track p.id) {
            <button type="button" class="btn" [disabled]="!pending()" (click)="mark(p.id)">
              {{ p.label }}{{ marks()[p.id] ? ' ✓' : '' }}
            </button>
          }
          <button type="button" class="btn ghost" [disabled]="!hasMarks()" (click)="marks.set({})" i18n="@@format.wizard.clearMarks">Clear marks</button>
        </div>
      }
    } @else if (mode() === 'timestamps') {
      <label class="lbl"><ng-container i18n="@@format.wizard.timestampFormat">Timestamp format</ng-container>
        <input class="input mono" data-testid="timestamp-format" [value]="tsFormat()" (input)="tsFormat.set($any($event.target).value)" placeholder="%Y/%m/%d,%H:%M:%S%.f%:z" />
      </label>
      <div class="status">
        @if (suggested()) {
          <ng-container i18n="@@format.wizard.timestampSuggested">Debrief checked this format against the lines it could not read.</ng-container>&ngsp;
        }
        <ng-container i18n="@@format.wizard.timestampHint">.NET style (dd/MM/yyyy HH:mm:ss.fff) or strftime (%d/%m/%Y %H:%M:%S%.f). It is added to the timestamp formats of the parser that reads this file.</ng-container>
      </div>
    } @else {
      <label class="lbl"><ng-container i18n="@@format.wizard.outputTemplate">Output template</ng-container>
        <input class="input mono" [value]="template()" (input)="template.set($any($event.target).value)" placeholder="{Timestamp:yyyy-MM-dd HH:mm:ss.fff zzz} [{Level:u3}] {Message:lj}{NewLine}{Exception}" />
      </label>
    }

    @if (mode() === 'marks') {
      <div class="status" i18n="@@format.wizard.generated">Generated pattern</div>
      <div class="box" data-testid="format-regex">{{ regex() ?? markHint }}</div>
      <div><debrief-ai-button label="Suggest regex with AI" i18n-label="@@format.wizard.aiLabel" hint="The local model proposes a pattern for these lines; the preview checks it" i18n-hint="@@format.wizard.aiHint" testId="ai-format-regex" [disabled]="!lines().length" (pressed)="suggestWithAi()" /></div>
    }

    @if (request()) {
      <div class="status" role="status" data-testid="format-summary">
        @if (previewError() ?? preview()?.error; as pe) { <span class="err">{{ pe }}</span> }
        @else if (!preview()) { <ng-container i18n="@@format.wizard.checking">Checking the sample…</ng-container> }
        @else { <ng-container i18n="@@format.wizard.summary"><strong>{{ preview()!.total }}</strong> events parsed from the sample, {{ preview()!.skipped }} lines skipped. Lines that do not start an event (such as stack traces) are attached to the previous event.</ng-container> }
      </div>
      @if (preview(); as p) {
        <div class="preview" role="table" aria-label="Parse preview" i18n-aria-label="@@format.wizard.previewLabel">
          <div class="prow head" role="row"><span role="columnheader" i18n="@@format.wizard.col.time">Time</span><span role="columnheader" i18n="Log severity level|Column header@@format.wizard.col.level">Level</span><span role="columnheader" i18n="@@format.wizard.col.message">Message</span></div>
          @for (r of p.records; track r.line) {
            <div class="prow" role="row">
              <span role="cell" class="mono">{{ time(r.ts) }}</span><span role="cell" class="mono">{{ r.level }}</span><span role="cell">{{ r.message }}</span>
            </div>
          }
        </div>
      }
    }

    @if (error(); as e) { <div class="err" role="alert">{{ e }}</div> }
    <div class="foot">
      <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@format.wizard.cancel">Cancel</button>
      <button type="button" class="btn primary" [disabled]="!canSave() || saving()" (click)="save()">{{ saving() ? applying : saveLabel }}</button>
    </div>
  `,
  styles: [
    DIALOG_FORM_STYLES,
    `
    .seg { display: flex; gap: 4px; }
    .box[role=textbox] { user-select: text; cursor: text; }
    .preview { max-height: 200px; overflow: auto; border: 1px solid var(--border); border-radius: 6px; font-size: var(--fs-sm); }
    .prow { display: grid; grid-template-columns: 220px 60px minmax(0, 1fr); gap: 8px; padding: 3px 8px; border-bottom: 1px solid var(--border); }
    .prow.head { position: sticky; top: 0; background: var(--bg-3); color: var(--muted); font-weight: 600; }
    .hl[data-part=timestamp] { background: color-mix(in srgb, var(--accent) 25%, transparent); }
    .hl[data-part=level] { background: color-mix(in srgb, var(--lvl-warn) 30%, transparent); }
    .hl[data-part=message] { background: color-mix(in srgb, var(--mk-start) 25%, transparent); }
  `,
  ],
})
export class FormatWizard implements OnInit {
  protected readonly ref = inject(DIALOG_REF);
  private readonly backend = inject(DebriefBackend);
  private readonly settings = inject(SettingsStore);
  private readonly store = inject(WorkspaceStore);
  private readonly parsers = inject(ParsersBackend);
  private readonly library = inject(ParsersStore);
  private readonly ai = inject(AiLauncher);

  readonly path = input.required<string>();
  readonly name = input.required<string>();
  /**
   * Opened because the file's timestamps were not read: the format the engine proposed (null: none). Set, the
   * wizard starts in "Timestamp format" mode; left undefined, that mode is not offered.
   */
  readonly timestampFormat = input<string | null | undefined>(undefined);

  private readonly sampleEl = viewChild<ElementRef<HTMLElement>>('sample');
  protected readonly parts = PARTS;
  protected readonly markHint = $localize`:@@format.wizard.markHint:Mark a timestamp and a message to generate a pattern.`;
  protected readonly applying = $localize`:@@format.wizard.applying:Applying…`;
  protected readonly saveLabel = $localize`:@@format.wizard.save:Save parser and re-read`;
  protected readonly mode = signal<'marks' | 'template' | 'timestamps'>('marks');
  /** The timestamp format being tried (starts as the engine's proposal). */
  protected readonly tsFormat = signal('');
  protected readonly suggested = computed(() => !!this.timestampFormat() && this.tsFormat().trim() === this.timestampFormat());
  /** Start of the file as the engine decoded it; this is also what the engine previews. */
  private readonly head = signal('');
  protected readonly lines = computed(() => (this.head() ? this.head().split(/\r?\n/).slice(0, PREVIEW_LINES).filter((l) => l.length > 0) : []));
  protected readonly preview = signal<ParsePreview | null>(null);
  protected readonly previewError = signal<string | null>(null);
  private previewSeq = 0;
  protected readonly loadError = signal<string | null>(null);
  protected readonly sampleIndex = signal(0);
  protected readonly marks = signal<Marks>({});
  protected readonly template = signal('');
  protected readonly pending = signal<{ start: number; end: number } | null>(null);
  protected readonly saving = signal(false);
  protected readonly error = signal<string | null>(null);

  protected readonly sample = computed(() => this.lines()[this.sampleIndex()] ?? '');
  protected readonly hasMarks = computed(() => Object.keys(this.marks()).length > 0);
  /** A pattern the local AI suggested (replaces the one generated from the marks until they change). */
  private readonly aiRegex = signal<string | null>(null);
  protected readonly regex = computed(() => (this.mode() === 'marks' ? (this.aiRegex() ?? regexFromMarks(this.sample(), this.marks())) : null));
  /** The parser that read this file (what a timestamp fix changes); null for a file no parser read. */
  protected readonly baseParser = computed(() => this.store.summary()?.sources.find((s) => s.path === this.path())?.parser ?? null);
  /** What the engine composes the candidate parser from; null until there is something to try. */
  protected readonly request = computed<ParserDraftRequest | null>(() => {
    const template = this.template().trim();
    const regex = this.regex();
    const tsFormat = this.tsFormat().trim();
    const fileName = this.name();
    if (this.mode() === 'timestamps') return tsFormat ? { fileName, base: this.baseParser(), regex: null, template: null, timestampFormat: tsFormat } : null;
    if (this.mode() === 'template') return template ? { fileName, base: null, regex: null, template: this.template(), timestampFormat: null } : null;
    return regex ? { fileName, base: null, regex, template: null, timestampFormat: null } : null;
  });
  /** The parser the engine composed for the current request: what Save adds to the library. */
  protected readonly candidate = signal<ParserDocDto | null>(null);
  protected readonly canSave = computed(() => !!this.request() && !!this.candidate() && !!this.preview() && !this.preview()!.error && !this.previewError() && this.preview()!.total > 0);
  protected readonly segments = computed(() => {
    const line = this.sample();
    const ranges = (Object.entries(this.marks()) as [FormatPart, { start: number; end: number }][]).sort((a, b) => a[1].start - b[1].start);
    const out: { text: string; part?: FormatPart }[] = [];
    let pos = 0;
    for (const [part, m] of ranges) {
      if (m.start > pos) out.push({ text: line.slice(pos, m.start) });
      out.push({ text: line.slice(m.start, m.end), part });
      pos = m.end;
    }
    if (pos < line.length) out.push({ text: line.slice(pos) });
    return out;
  });

  constructor() {
    // Ask the engine for the candidate parser and how it reads the sample (after a short pause while typing).
    // effect-state: debounced IPC preview; clears the preview when there is no sample
    effect((onCleanup) => {
      const request = this.request();
      const head = this.head();
      if (!request || !head) {
        untracked(() => {
          this.previewSeq++;
          this.candidate.set(null);
          this.preview.set(null);
          this.previewError.set(null);
        });
        return;
      }
      const timer = setTimeout(() => untracked(() => void this.runPreview(request, head)), PREVIEW_DEBOUNCE_MS);
      onCleanup(() => clearTimeout(timer));
    });
  }

  ngOnInit(): void {
    const proposed = this.timestampFormat();
    if (proposed !== undefined) {
      this.mode.set('timestamps');
      this.tsFormat.set(proposed ?? '');
    }
    void this.loadHead();
  }

  protected async loadHead(): Promise<void> {
    this.loadError.set(null);
    try {
      this.head.set(await this.backend.readFileHead(this.path(), HEAD_BYTES));
    } catch (e) {
      this.loadError.set(describeError(e));
    }
  }

  private async runPreview(request: ParserDraftRequest, sample: string): Promise<void> {
    const mine = ++this.previewSeq;
    this.candidate.set(null);
    this.preview.set(null);
    this.previewError.set(null);
    try {
      const draft = await this.parsers.draft(request);
      if (mine !== this.previewSeq) return;
      const result = await this.backend.previewParse({ toml: draft.toml, sample, fileName: this.name(), limit: PREVIEW_LINES });
      if (mine !== this.previewSeq) return;
      this.candidate.set(draft);
      this.preview.set(result);
    } catch (e) {
      if (mine === this.previewSeq) this.previewError.set(describeError(e));
    }
  }

  protected time(ts: number | null): string {
    return ts === null ? '–' : formatFull(ts, this.store.timezone(), this.settings.dateFormat());
  }

  protected onSelection(): void {
    const el = this.sampleEl()?.nativeElement;
    const sel = document.getSelection();
    if (!el || !sel?.rangeCount || !el.contains(sel.anchorNode) || !el.contains(sel.focusNode)) return;
    const s = selectionWithin(el, sel.getRangeAt(0));
    if (s) this.pending.set({ start: s.start, end: s.end });
  }

  protected pickSample(index: string): void {
    this.sampleIndex.set(Number(index));
    this.marks.set({});
    this.pending.set(null);
  }

  protected mark(part: FormatPart): void {
    const range = this.pending();
    if (!range) return;
    // A new mark replaces any overlapping mark of another part.
    const next: Marks = {};
    for (const [p, m] of Object.entries(this.marks()) as [FormatPart, { start: number; end: number }][]) {
      if (p !== part && (m.end <= range.start || m.start >= range.end)) next[p] = m;
    }
    next[part] = range;
    this.marks.set(next);
    this.pending.set(null);
    document.getSelection()?.removeAllRanges();
  }

  protected async save(): Promise<void> {
    const candidate = this.candidate();
    if (!candidate || !this.canSave()) return;
    const base = this.request()?.base ?? null;
    this.saving.set(true);
    this.error.set(null);
    try {
      // A timestamp fix replaces the file's parser (a built-in's becomes an override); a new format is a new parser.
      await this.library.save(candidate.toml, { previousId: base, overwrite: base !== null });
      void this.library.reread();
      const id = candidate.id;
      this.store.setNotice($localize`:@@format.wizard.saved:Saved the parser ${id}:id:; the files are read again.`);
      this.ref.close(true);
    } catch (e) {
      this.error.set(describeError(e));
    } finally {
      this.saving.set(false);
    }
  }

  /** "Suggest regex" with the local AI model, from the sample lines. */
  protected async suggestWithAi(): Promise<void> {
    const p = await this.ai.suggestRegex(this.lines());
    if (p) this.aiRegex.set(p);
  }
}
