import { ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, input, OnInit, signal, untracked, viewChild } from '@angular/core';
import { selectionWithin } from '../context/selection';
import { ParsePreview } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { formatFull } from '../core/format';
import { describeError } from '../core/logger';
import { ProfileModel } from '../core/profile.model';
import { WorkspaceStore } from '../core/workspace.store';
import { SettingsStore } from '../settings/settings.store';
import { ProfileEdits } from '../profile/profile-edits';
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
  { id: 'timestamp', label: 'Timestamp' },
  { id: 'level', label: 'Level' },
  { id: 'message', label: 'Message' },
];

/** "Set up format": teaches IMT how to read a file it did not recognise, from marks on a sample line or a Serilog template. */
@Component({
  selector: 'imt-format-wizard',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [HelpHint],
  host: { '(document:selectionchange)': 'onSelection()' },
  template: `
    <p>IMT did not recognise <span class="mono">{{ name() }}</span>. Show it how a log line is built and it will read this kind of file from now on. <imt-help label="Format wizard" text="Select the timestamp, then the level, then the message on a sample line and press the matching button. IMT builds a regular expression from your marks and shows how the first lines parse." /></p>

    @if (loadError(); as e) {
      <div class="err" role="alert">{{ e }} <button type="button" class="btn" (click)="loadHead()">Retry</button></div>
    }

    <div class="seg" role="group" aria-label="How to describe the format">
      <button type="button" class="btn" [class.active]="mode() === 'marks'" [attr.aria-pressed]="mode() === 'marks'" (click)="mode.set('marks')">Mark on a sample line</button>
      <button type="button" class="btn" [class.active]="mode() === 'template'" [attr.aria-pressed]="mode() === 'template'" (click)="mode.set('template')">Serilog output template</button>
    </div>

    @if (mode() === 'marks') {
      @if (lines().length) {
        <label class="lbl">Sample line
          <select class="select mono" [value]="sampleIndex()" (change)="pickSample($any($event.target).value)">
            @for (l of lines(); track $index) { <option [value]="$index" [selected]="$index === sampleIndex()">{{ $index + 1 }}: {{ l.slice(0, 80) }}</option> }
          </select>
        </label>
        <div class="status">Select part of the line with the mouse or Shift+arrows, then press a button:</div>
        <div #sample class="box" tabindex="0" role="textbox" aria-readonly="true" aria-label="Sample line" data-testid="sample-line">
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
          <button type="button" class="btn ghost" [disabled]="!hasMarks()" (click)="marks.set({})">Clear marks</button>
        </div>
      }
    } @else {
      <label class="lbl">Output template
        <input class="input mono" [value]="template()" (input)="template.set($any($event.target).value)" placeholder="{Timestamp:yyyy-MM-dd HH:mm:ss.fff zzz} [{Level:u3}] {Message:lj}{NewLine}{Exception}" />
      </label>
    }

    @if (mode() === 'marks') {
      <div class="status">Generated pattern</div>
      <div class="box" data-testid="format-regex">{{ regex() ?? 'Mark a timestamp and a message to generate a pattern.' }}</div>
    }

    @if (mutation()) {
      <div class="status" role="status" data-testid="format-summary">
        @if (previewError() ?? preview()?.error; as pe) { <span class="err">{{ pe }}</span> }
        @else if (!preview()) { Checking the sample… }
        @else { <strong>{{ preview()!.total }}</strong> events parsed from the sample, {{ preview()!.skipped }} lines skipped. Lines that do not start an event (such as stack traces) are attached to the previous event. }
      </div>
      @if (preview(); as p) {
        <div class="preview" role="table" aria-label="Parse preview">
          <div class="prow head" role="row"><span role="columnheader">Time</span><span role="columnheader">Level</span><span role="columnheader">Message</span></div>
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
      <button type="button" class="btn ghost" (click)="ref.close()">Cancel</button>
      <button type="button" class="btn primary" [disabled]="!canSave() || saving()" (click)="save()">{{ saving() ? 'Applying…' : 'Save and re-import' }}</button>
    </div>
  `,
  styles: [
    DIALOG_FORM_STYLES,
    `
    .seg { display: flex; gap: 4px; }
    .box[role=textbox] { user-select: text; cursor: text; }
    .preview { max-height: 200px; overflow: auto; border: 1px solid var(--border); border-radius: 6px; font-size: 12px; }
    .prow { display: grid; grid-template-columns: 220px 60px minmax(0, 1fr); gap: 8px; padding: 3px 8px; border-bottom: 1px solid var(--border); }
    .prow.head { position: sticky; top: 0; background: var(--bg-3); color: var(--muted); font-weight: 600; }
    .hl[data-part=timestamp] { background: rgba(76, 141, 255, 0.25); }
    .hl[data-part=level] { background: rgba(233, 174, 63, 0.3); }
    .hl[data-part=message] { background: rgba(63, 191, 127, 0.25); }
  `,
  ],
})
export class FormatWizard implements OnInit {
  protected readonly ref = inject(DIALOG_REF);
  private readonly backend = inject(ImtBackend);
  private readonly settings = inject(SettingsStore);
  private readonly edits = inject(ProfileEdits);
  private readonly store = inject(WorkspaceStore);

  readonly path = input.required<string>();
  readonly name = input.required<string>();

  private readonly sampleEl = viewChild<ElementRef<HTMLElement>>('sample');
  protected readonly parts = PARTS;
  protected readonly mode = signal<'marks' | 'template'>('marks');
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
  protected readonly regex = computed(() => (this.mode() === 'marks' ? regexFromMarks(this.sample(), this.marks()) : null));
  /** The change to the profile this wizard would save; also what the engine previews. Null until there is something to try. */
  protected readonly mutation = computed<((m: ProfileModel) => void) | null>(() => {
    const template = this.template().trim();
    const regex = this.regex();
    // New formats go first so they win over the generic patterns.
    if (this.mode() === 'template' && template) {
      return (m) => {
        m.parse = { ...(m.parse ?? {}) };
        m.parse.output_templates = [this.template(), ...(m.parse.output_templates ?? [])];
      };
    }
    if (this.mode() === 'marks' && regex) {
      return (m) => {
        m.parse = { ...(m.parse ?? {}) };
        m.parse.regexes = [regex, ...(m.parse.regexes ?? [])];
      };
    }
    return null;
  });
  protected readonly canSave = computed(() => !!this.mutation() && !!this.preview() && !this.preview()!.error && !this.previewError() && this.preview()!.total > 0);
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
    // Ask the engine how the candidate profile reads the sample (after a short pause while typing).
    effect((onCleanup) => {
      const mutation = this.mutation();
      const head = this.head();
      if (!mutation || !head) {
        untracked(() => {
          this.previewSeq++;
          this.preview.set(null);
          this.previewError.set(null);
        });
        return;
      }
      const timer = setTimeout(() => untracked(() => void this.runPreview(mutation, head)), PREVIEW_DEBOUNCE_MS);
      onCleanup(() => clearTimeout(timer));
    });
  }

  ngOnInit(): void {
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

  private async runPreview(mutation: (m: ProfileModel) => void, sample: string): Promise<void> {
    const mine = ++this.previewSeq;
    this.preview.set(null);
    this.previewError.set(null);
    try {
      const { model } = await this.edits.current();
      mutation(model);
      const result = await this.backend.previewParse({ toml: this.edits.toToml(model), sample, fileName: this.name(), limit: PREVIEW_LINES });
      if (mine === this.previewSeq) this.preview.set(result);
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
    const mutation = this.mutation();
    if (!mutation) return;
    this.saving.set(true);
    this.error.set(null);
    try {
      await this.edits.apply(mutation);
      this.store.setNotice('Saved the format; files were re-imported.');
      this.ref.close(true);
    } catch (e) {
      this.error.set(describeError(e));
    } finally {
      this.saving.set(false);
    }
  }
}
