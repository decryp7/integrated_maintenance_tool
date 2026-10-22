import { ChangeDetectionStrategy, Component, computed, DestroyRef, inject, OnInit, signal } from '@angular/core';
import { RedactedCopyChoices, RedactedCopyOptions, RedactedCopyPreview, RedactedExport, RedactMode } from '../../core/api.types';
import { Platform } from '../../core/platform';
import { UiStateRegistry } from '../../lifecycle/ui-state';
import { ProjectService, PROJECT_EXTENSION } from '../../projects/project.service';
import { formatBytes, formatCount } from '../../shared/format/units';
import { errorMessage } from '../../tools/tool-errors';
import { DIALOG_REF } from '../../ui/dialog.service';
import { OverwriteGuard } from '../../ui/overwrite-guard';
import { ReportStore } from '../report/report.store';
import { ShareBackend } from '../share-backend';
import { detectorLabel, modeChoices } from './redacted-copy-labels';

/** Wait after the last change before the preview is asked again. */
export const PREVIEW_DEBOUNCE_MS = 200;

/**
 * File ▸ "Share redacted copy…": pick what to redact, see what it would change and what is left out, then write the
 * copy as a project file. The engine builds and redacts everything; this only collects the choices and shows the answers.
 */
@Component({
  selector: 'debrief-redacted-copy-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <p class="intro" i18n="@@share.redactedCopy.dialog.intro">
      Creates a copy of this project (.debriefz) that you can share. It is built from the redacted text of every text source (JSON and text logs), so its events, field statistics, search index, notes, AI chats,
      saved queries and reports contain none of the values the selected rules find.
    </p>
    @if (loadError(); as e) {
      <p class="err" role="alert" data-testid="rc-load-error">{{ e }}</p>
    }
    @if (result(); as r) {
      <section class="result" data-testid="rc-result" aria-live="polite">
        <h3 i18n="@@share.redactedCopy.result.title">Copy created</h3>
        <p data-testid="rc-result-summary" i18n="@@share.redactedCopy.result.summary">{{ r.sources }} sources and {{ count(r.events) }} events written ({{ size(r.bytes) }}).</p>
        <ul class="hits" aria-label="Values redacted per rule" i18n-aria-label="@@share.redactedCopy.result.hitsLabel">
          @for (h of r.hits; track h.rule) {
            <li>
              <span>{{ label(h.rule) }}</span
              ><b>{{ count(h.count) }}</b>
            </li>
          }
        </ul>
        @if (r.excluded.length) {
          <p class="warn" i18n="@@share.redactedCopy.result.excluded">Left out of the copy:</p>
          <ul data-testid="rc-result-excluded">
            @for (x of r.excluded; track x.name) {
              <li>
                <b>{{ x.name }}</b> — {{ x.reason }}
              </li>
            }
          </ul>
        }
      </section>
      <div class="foot">
        <button type="button" class="btn primary" data-testid="rc-done" (click)="ref.close(true)" i18n="@@share.redactedCopy.done">Done</button>
      </div>
    } @else {
      <div class="cols">
        <fieldset>
          <legend i18n="@@share.redactedCopy.detectors">What to redact</legend>
          <div class="list" data-testid="rc-detectors">
            @for (d of detectors(); track d) {
              <label class="choice">
                <input type="checkbox" [checked]="pickedDetectors().has(d)" [disabled]="busy()" (change)="toggleDetector(d)" [attr.data-detector]="d" />
                <span>{{ label(d) }}</span>
              </label>
            }
            @for (r of rules(); track r) {
              <label class="choice">
                <input type="checkbox" [checked]="pickedRules().has(r)" [disabled]="busy()" (change)="toggleRule(r)" [attr.data-rule]="r" />
                <span>{{ r }}</span
                ><span class="hint" i18n="@@share.redactedCopy.customRule">custom rule</span>
              </label>
            }
          </div>
        </fieldset>
        <fieldset>
          <legend i18n="@@share.redactedCopy.mode">How to replace values</legend>
          @for (m of modes; track m.mode) {
            <label class="choice">
              <input type="radio" name="debrief-rc-mode" [value]="m.mode" [checked]="mode() === m.mode" [disabled]="busy()" (change)="setMode(m.mode)" [attr.data-mode]="m.mode" />
              <span class="name">{{ m.label }}</span>
              <span class="hint">{{ m.hint }}</span>
            </label>
          }
        </fieldset>
      </div>

      <section class="preview" aria-live="polite" data-testid="rc-preview">
        <h3 i18n="@@share.redactedCopy.preview.title">Preview</h3>
        @if (previewError(); as e) {
          <p class="err" role="alert" data-testid="rc-preview-error">{{ e }}</p>
        } @else if (preview(); as p) {
          <p data-testid="rc-scanned" [class.stale]="previewing()" i18n="@@share.redactedCopy.preview.scanned">Scanned {{ p.summary.scanned }} events, {{ p.summary.changed }} would change.</p>
          @if (p.summary.truncated) {
            <p class="hint" i18n="@@share.redactedCopy.preview.truncated">Only the first events were scanned for this preview; the copy covers all of them.</p>
          }
          <table data-testid="rc-hits">
            <caption class="sr-only" i18n="@@share.redactedCopy.preview.hitsCaption">Values found per rule</caption>
            <tbody>
              @for (h of p.summary.hits; track h.rule) {
                <tr [attr.data-rule-hit]="h.rule">
                  <th scope="row">{{ label(h.rule) }}</th>
                  <td>{{ count(h.count) }}</td>
                </tr>
              }
            </tbody>
          </table>
          @if (p.summary.samples.length) {
            <ul class="samples" data-testid="rc-samples">
              @for (s of p.summary.samples; track s.recordId) {
                <li>
                  <code class="before" i18n-title="@@share.redactedCopy.preview.before" title="Before">{{ s.before }}</code>
                  <code class="after" i18n-title="@@share.redactedCopy.preview.after" title="After">{{ s.after }}</code>
                </li>
              }
            </ul>
          }
          @if (p.excluded.length) {
            <div class="leftout" data-testid="rc-excluded">
              <b i18n="@@share.redactedCopy.preview.leftOut">Left out of the copy</b>
              <ul>
                @for (x of p.excluded; track x.name) {
                  <li>
                    <b>{{ x.name }}</b> — {{ x.reason }}
                  </li>
                }
              </ul>
            </div>
          }
        } @else {
          <p class="hint" i18n="@@share.redactedCopy.preview.loading">Looking at your events…</p>
        }
      </section>

      <ul class="notes" data-testid="rc-notes">
        <li i18n="@@share.redactedCopy.note.sources">Sources in formats that cannot be redacted (binary formats such as event logs and dumps, CSV, XML) are left out and listed.</li>
        <li i18n="@@share.redactedCopy.note.pictures">Note images, attachments and video frames are not included: pictures cannot be redacted.</li>
        <li i18n="@@share.redactedCopy.note.profile">The profile's own redaction rules are not carried into the copy. Clock corrections are kept.</li>
      </ul>

      @if (createError(); as e) {
        <p class="err" role="alert" data-testid="rc-create-error">{{ e }}</p>
      }
      <div class="foot">
        <button type="button" class="btn ghost" data-testid="rc-cancel" (click)="ref.close()" i18n="@@share.redactedCopy.cancel">Cancel</button>
        <button type="button" class="btn primary" data-testid="rc-create" [disabled]="!canCreate()" [attr.aria-busy]="busy()" (click)="create()">
          @if (busy()) {
            <span i18n="@@share.redactedCopy.creating">Creating the copy…</span>
          } @else if (createError()) {
            <span i18n="@@share.redactedCopy.retry">Try again</span>
          } @else {
            <span i18n="@@share.redactedCopy.create">Create copy…</span>
          }
        </button>
      </div>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; }
    p, h3 { margin: 0; }
    h3 { font-size: var(--fs-sm); }
    .hint { color: var(--muted); font-size: var(--fs-xs); }
    .cols { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
    fieldset { border: 0; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; min-width: 0; }
    legend { font-weight: 600; padding: 0; margin-bottom: 4px; }
    .list { max-height: 220px; overflow: auto; display: flex; flex-direction: column; gap: 2px; border: 1px solid var(--border); border-radius: 6px; padding: 4px; }
    .choice { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 8px; padding: 3px 6px; border-radius: 5px; cursor: pointer; }
    .choice:hover { background: var(--bg-hover); }
    .name { font-weight: 500; }
    .preview { border: 1px solid var(--border); border-radius: 6px; padding: 8px; display: flex; flex-direction: column; gap: 6px; }
    .stale { opacity: 0.6; }
    table { border-collapse: collapse; font-size: var(--fs-xs); }
    th, td { text-align: left; padding: 1px 12px 1px 0; font-weight: 400; }
    td { font-variant-numeric: tabular-nums; }
    .samples { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
    .samples li { display: grid; gap: 2px; }
    code { font-family: var(--font-mono, monospace); font-size: var(--fs-xs); white-space: pre-wrap; overflow-wrap: anywhere; }
    .before { color: var(--muted); }
    .leftout, .warn { border-left: 3px solid var(--warning, #c90); padding-left: 8px; }
    .leftout ul, .notes, .result ul { margin: 2px 0 0; padding-left: 18px; }
    .notes { color: var(--muted); font-size: var(--fs-xs); }
    .hits { list-style: none; padding: 0; }
    .hits li { display: flex; gap: 12px; }
    .err { color: var(--danger-text); }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
    .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
  `,
})
export class RedactedCopyDialog implements OnInit {
  protected readonly count = formatCount;
  protected readonly ref = inject(DIALOG_REF);
  private readonly backend = inject(ShareBackend);
  private readonly platform = inject(Platform);
  private readonly project = inject(ProjectService);
  private readonly registry = inject(UiStateRegistry);
  private readonly reports = inject(ReportStore);
  private readonly overwrite = inject(OverwriteGuard);
  private readonly destroyRef = inject(DestroyRef);

  protected readonly modes = modeChoices();
  protected readonly detectors = signal<string[]>([]);
  protected readonly rules = signal<string[]>([]);
  protected readonly pickedDetectors = signal<ReadonlySet<string>>(new Set());
  protected readonly pickedRules = signal<ReadonlySet<string>>(new Set());
  protected readonly mode = signal<RedactMode>('mask');
  protected readonly loadError = signal<string | null>(null);
  protected readonly preview = signal<RedactedCopyPreview | null>(null);
  protected readonly previewError = signal<string | null>(null);
  protected readonly previewing = signal(false);
  protected readonly busy = signal(false);
  protected readonly createError = signal<string | null>(null);
  protected readonly result = signal<RedactedExport | null>(null);
  private readonly ready = signal(false);
  private timer: ReturnType<typeof setTimeout> | undefined;
  private generation = 0;

  /** The copy can be created when the engine accepted the picks and the preview is current. */
  protected readonly canCreate = computed(() => this.ready() && !this.busy() && !this.previewing() && !this.previewError() && this.preview() !== null);

  ngOnInit(): void {
    this.destroyRef.onDestroy(() => clearTimeout(this.timer));
    this.backend.redactedCopyChoices().then(
      (c) => this.applyChoices(c),
      (e: unknown) => this.loadError.set(errorMessage(e)),
    );
  }

  private applyChoices(c: RedactedCopyChoices): void {
    this.detectors.set(c.detectors);
    this.rules.set(c.rules);
    this.pickedDetectors.set(new Set(c.selected.detectors));
    this.pickedRules.set(new Set(c.selected.rule.map((r) => r.name)));
    this.mode.set(c.selected.mode);
    this.ready.set(true);
    this.refresh(0);
  }

  protected label(rule: string): string {
    return detectorLabel(rule);
  }

  protected size(bytes: number): string {
    return formatBytes(bytes);
  }

  protected toggleDetector(id: string): void {
    this.pickedDetectors.update((s) => toggled(s, id));
    this.refresh();
  }

  protected toggleRule(name: string): void {
    this.pickedRules.update((s) => toggled(s, name));
    this.refresh();
  }

  protected setMode(mode: RedactMode): void {
    this.mode.set(mode);
    this.refresh();
  }

  private options(): RedactedCopyOptions {
    return {
      detectors: this.detectors().filter((d) => this.pickedDetectors().has(d)),
      rules: this.rules().filter((r) => this.pickedRules().has(r)),
      mode: this.mode(),
    };
  }

  /** Asks the engine again after the last change (older answers that arrive late are ignored). */
  private refresh(delay = PREVIEW_DEBOUNCE_MS): void {
    clearTimeout(this.timer);
    const generation = ++this.generation;
    this.previewing.set(true);
    this.timer = setTimeout(() => {
      this.backend.redactedCopyPreview(this.options()).then(
        (p) => {
          if (generation !== this.generation) return;
          this.preview.set(p);
          this.previewError.set(null);
          this.previewing.set(false);
        },
        (e: unknown) => {
          if (generation !== this.generation) return;
          this.preview.set(null);
          this.previewError.set(errorMessage(e));
          this.previewing.set(false);
        },
      );
    }, delay);
  }

  /** Asks where to write the copy (like Save project as…), writes it, shows what it holds. */
  protected async create(): Promise<void> {
    this.createError.set(null);
    this.busy.set(true);
    try {
      const path = await this.platform.pickSavePath($localize`:@@share.redactedCopy.pickTitle:Save redacted copy`, `${this.project.name()}-redacted${PROJECT_EXTENSION}`, [
        { name: $localize`:@@share.redactedCopy.filter:Debrief project`, extensions: ['debriefz'] },
      ]);
      if (!path) return;
      const options = this.options();
      const done = await this.overwrite.write(path, (overwrite) =>
        this.backend.exportRedactedCopy({ path, name: this.project.name(), options, ui: this.registry.serialize(), reports: this.reports.reports(), overwrite }),
      );
      if (done) this.result.set(done);
    } catch (e) {
      this.createError.set(errorMessage(e));
    } finally {
      this.busy.set(false);
    }
  }
}

function toggled(set: ReadonlySet<string>, id: string): ReadonlySet<string> {
  const next = new Set(set);
  if (!next.delete(id)) next.add(id);
  return next;
}
