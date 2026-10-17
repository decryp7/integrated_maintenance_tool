import { ChangeDetectionStrategy, Component, computed, effect, inject, input, OnInit, signal, untracked } from '@angular/core';
import { ParsePreview, ProfileTemplate } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { formatFull } from '../core/format';
import { describeError } from '../core/logger';
import { UxBackend } from '../core/ux-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { SettingsStore } from '../settings/settings.store';
import { DIALOG_REF } from '../ui/dialog.service';
import { ProfileUndo } from '../undo/profile-undo';
import { bestTemplate, groupTemplates, ratioLabel, readRatio } from './templates';

/** How much of the user's file the engine previews. */
export const TEMPLATE_HEAD_BYTES = 32 * 1024;
/** Parsed events shown in the preview. */
export const TEMPLATE_PREVIEW_ROWS = 20;

/**
 * Ready-made profiles for common log formats, with a live preview of how each one reads the user's file
 * (or the template's own sample). "Use this" validates and saves it as the active profile (undoable).
 */
@Component({
  selector: 'imt-templates-gallery',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <p class="intro">
      Pick the format your logs are written in. @if (name()) { The preview shows how each template reads <span class="mono">{{ name() }}</span>. } @else { The preview uses a short sample of each format. }
    </p>
    @if (loadError(); as e) {
      <div class="err" role="alert">Could not load the templates: {{ e }} <button type="button" class="btn" (click)="load()">Retry</button></div>
    } @else if (!templates()) {
      <div class="muted" role="status">Loading templates…</div>
    } @else if (!templates()!.length) {
      <div class="muted">No templates are installed with this version of IMT.</div>
    } @else {
      <div class="cols">
        <nav class="list" aria-label="Templates">
          @for (g of groups(); track g.category) {
            <h3>{{ g.category }}</h3>
            <ul>
              @for (t of g.templates; track t.id) {
                <li>
                  <button type="button" class="tpl" [class.on]="t.id === selectedId()" [attr.aria-pressed]="t.id === selectedId()" [attr.data-template]="t.id" (click)="select(t.id)">
                    <span class="tname">{{ t.name }}</span>
                    @if (ratios().has(t.id)) {
                      <span class="badge" [class.good]="ratios().get(t.id)! >= 0.8">{{ ratioText(t.id) }}</span>
                    }
                  </button>
                </li>
              }
            </ul>
          }
        </nav>
        @if (selected(); as t) {
          <section class="detail" aria-label="Template preview">
            <h3 class="title">{{ t.name }}</h3>
            <p class="desc">{{ t.description }}</p>
            @if (headError(); as e) {
              <div class="err" role="alert">Could not read {{ name() }}: {{ e }}. Showing the template's own sample. <button type="button" class="btn" (click)="loadHead()">Retry</button></div>
            }
            <div class="status" role="status" data-testid="template-preview-status">
              @if (previewing()) {
                Previewing…
              } @else if (previewError(); as e) {
                <span class="err">Preview failed: {{ e }}</span>
              } @else if (preview(); as p) {
                @if (p.error) {
                  <span class="err">{{ p.error }}</span>
                } @else if (!p.total) {
                  <span class="err">This template reads no events from {{ sampleLabel() }}{{ p.skipped ? ' (' + p.skipped + ' lines not understood)' : '' }}. Try another one.</span>
                } @else {
                  Reads <strong>{{ p.total }}</strong> event{{ p.total === 1 ? '' : 's' }} from {{ sampleLabel() }}@if (p.skipped) {; {{ p.skipped }} line{{ p.skipped === 1 ? '' : 's' }} not understood}.
                }
              }
            </div>
            @if (preview()?.records?.length) {
              <div class="table-wrap">
                <table aria-label="Preview of the parsed events">
                  <thead><tr><th scope="col">Time</th><th scope="col">Level</th><th scope="col">Message</th></tr></thead>
                  <tbody>
                    @for (r of preview()!.records; track $index) {
                      <tr><td class="mono">{{ time(r.ts) }}</td><td>{{ r.level }}</td><td class="msg">{{ r.message }}</td></tr>
                    }
                  </tbody>
                </table>
              </div>
            }
            @if (applyError(); as e) {
              <div class="err" role="alert">The profile was not applied: {{ e }}</div>
            }
            <p class="note muted">Using a template replaces the active profile. You can undo it (Edit › Undo) or restore the previous one from the profile's History.</p>
            <div class="foot">
              <button type="button" class="btn ghost" (click)="ref.close(false)">Cancel</button>
              <button type="button" class="btn primary" [disabled]="applying()" (click)="use(t)">{{ applying() ? 'Applying…' : 'Use this' }}</button>
            </div>
          </section>
        }
      </div>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; min-height: 0; }
    p { margin: 0; color: var(--text-2); }
    .muted { color: var(--muted); font-size: 12px; }
    .err { color: var(--danger-text); font-size: 12px; }
    .cols { display: grid; grid-template-columns: 240px 1fr; gap: 14px; min-height: 0; }
    .list { max-height: 520px; overflow: auto; border-right: 1px solid var(--border); padding-right: 8px; }
    .list h3 { margin: 8px 0 4px; font-size: 11px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); font-weight: 600; }
    .list ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
    .tpl { width: 100%; display: flex; justify-content: space-between; align-items: center; gap: 6px; padding: 5px 8px; border: 0; border-radius: 5px; background: transparent; color: var(--text-2); cursor: pointer; text-align: left; font: inherit; }
    .tpl:hover { background: var(--bg-hover); color: var(--text); }
    .tpl.on { background: var(--bg-active); color: var(--text); box-shadow: inset 2px 0 0 var(--accent); }
    .tpl:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
    .badge { flex: none; font-size: 10px; padding: 0 5px; border-radius: 7px; border: 1px solid var(--border-3); color: var(--muted); }
    .badge.good { border-color: var(--ok); color: var(--text); }
    .detail { display: flex; flex-direction: column; gap: 8px; min-width: 0; }
    .title { margin: 0; font-size: 14px; }
    .status { font-size: 12px; color: var(--text-2); min-height: 16px; }
    .table-wrap { max-height: 300px; overflow: auto; border: 1px solid var(--border); border-radius: 6px; }
    table { width: 100%; border-collapse: collapse; font-size: 12px; }
    th { position: sticky; top: 0; background: var(--bg-3); text-align: left; font-weight: 600; color: var(--muted); padding: 4px 8px; }
    td { padding: 3px 8px; border-top: 1px solid var(--border); vertical-align: top; color: var(--text-2); }
    td.mono { white-space: nowrap; }
    .msg { word-break: break-word; }
    .note { font-size: 11px; }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class TemplatesGallery implements OnInit {
  protected readonly ref = inject(DIALOG_REF);
  private readonly ux = inject(UxBackend);
  private readonly backend = inject(ImtBackend);
  private readonly undo = inject(ProfileUndo);
  private readonly store = inject(WorkspaceStore);
  private readonly settings = inject(SettingsStore);

  /** A workspace file (opened or unrecognised) to preview against; null = the templates' own samples. */
  readonly path = input<string | null>(null);
  readonly name = input<string | null>(null);

  protected readonly templates = signal<ProfileTemplate[] | null>(null);
  protected readonly loadError = signal<string | null>(null);
  protected readonly selectedId = signal<string | null>(null);
  /** Start of the user's file; null until read (or when there is none). */
  private readonly head = signal<string | null>(null);
  protected readonly headError = signal<string | null>(null);
  protected readonly ratios = signal<ReadonlyMap<string, number>>(new Map());
  protected readonly preview = signal<ParsePreview | null>(null);
  protected readonly previewError = signal<string | null>(null);
  protected readonly previewing = signal(false);
  protected readonly applying = signal(false);
  protected readonly applyError = signal<string | null>(null);
  private previewSeq = 0;

  protected readonly groups = computed(() => groupTemplates(this.templates() ?? []));
  protected readonly selected = computed(() => this.templates()?.find((t) => t.id === this.selectedId()) ?? null);
  protected readonly sampleLabel = computed(() => (this.head() !== null ? `the start of ${this.name() ?? 'the file'}` : 'its sample'));

  constructor() {
    effect(() => {
      const t = this.selected();
      const head = this.head();
      if (t) untracked(() => void this.runPreview(t, head));
    });
  }

  ngOnInit(): void {
    void this.load();
  }

  async load(): Promise<void> {
    this.loadError.set(null);
    try {
      const list = await this.ux.profileTemplates();
      this.templates.set(list);
      if (this.path()) await this.loadHead();
      if (!this.selectedId()) this.selectedId.set(bestTemplate(this.ratios()) ?? list[0]?.id ?? null);
    } catch (e) {
      this.loadError.set(describeError(e));
    }
  }

  /** Reads the user's file and rates every template against it (the best one is selected). */
  async loadHead(): Promise<void> {
    const path = this.path();
    if (!path) return;
    this.headError.set(null);
    try {
      const head = await this.backend.readFileHead(path, TEMPLATE_HEAD_BYTES);
      const list = this.templates() ?? [];
      const rated = await Promise.all(
        list.map(async (t) => {
          try {
            return [t.id, readRatio(await this.backend.previewParse({ toml: t.toml, sample: head, fileName: this.name() ?? t.sampleName, limit: 1 }))] as const;
          } catch {
            return [t.id, 0] as const;
          }
        }),
      );
      this.ratios.set(new Map(rated));
      this.head.set(head);
      const best = bestTemplate(this.ratios());
      if (best) this.selectedId.set(best);
    } catch (e) {
      this.headError.set(describeError(e));
    }
  }

  protected select(id: string): void {
    this.selectedId.set(id);
    this.applyError.set(null);
  }

  protected ratioText(id: string): string {
    return ratioLabel(this.ratios().get(id) ?? 0);
  }

  protected time(ts: number | null): string {
    return formatFull(ts, this.store.timezone(), this.settings.dateFormat());
  }

  private async runPreview(t: ProfileTemplate, head: string | null): Promise<void> {
    const mine = ++this.previewSeq;
    this.previewing.set(true);
    this.previewError.set(null);
    try {
      const sample = head ?? t.sampleText;
      if (!sample) {
        // Binary formats (Windows .evtx) have no text sample: only a file of the user's can be previewed.
        if (mine === this.previewSeq) {
          this.preview.set(null);
          this.previewError.set(`${t.name} reads a binary format, so there is no text sample to preview. Open one of your ${t.sampleName.split('.').pop()} files, then choose “Start from a template” on it to preview.`);
        }
        return;
      }
      const fileName = head !== null ? (this.name() ?? t.sampleName) : t.sampleName;
      const p = await this.backend.previewParse({ toml: t.toml, sample, fileName, limit: TEMPLATE_PREVIEW_ROWS });
      if (mine === this.previewSeq) this.preview.set(p);
    } catch (e) {
      if (mine === this.previewSeq) {
        this.preview.set(null);
        this.previewError.set(describeError(e));
      }
    } finally {
      if (mine === this.previewSeq) this.previewing.set(false);
    }
  }

  /** Validates the template in the engine, then saves it as the active profile (undoable). */
  protected async use(t: ProfileTemplate): Promise<void> {
    this.applying.set(true);
    this.applyError.set(null);
    try {
      const v = await this.backend.validateProfile(t.toml);
      if (!v.ok) {
        this.applyError.set(`${v.line ? `line ${v.line}: ` : ''}${v.error ?? 'the profile is not valid'}`);
        return;
      }
      const summary = await this.undo.save('profile template', t.toml);
      this.store.setNotice(`Applied the ${t.name} template: ${summary.total.toLocaleString('en-US')} events`);
      this.ref.close(true);
    } catch (e) {
      this.applyError.set(describeError(e));
    } finally {
      this.applying.set(false);
    }
  }
}
