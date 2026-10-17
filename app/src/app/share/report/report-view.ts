import { ChangeDetectionStrategy, Component, computed, ElementRef, inject } from '@angular/core';
import { WorkspaceStore } from '../../core/workspace.store';
import { BookmarkStore } from '../../projects/bookmarks/bookmark.store';
import { QuerySession } from '../../analysis/query-session';
import { TimelineAnnotations } from '../../timeline/annotations.store';
import { Icon } from '../../shared/icon';
import { DialogService } from '../../ui/dialog.service';
import { MarkdownView } from './markdown-view';
import { ReportActions } from './report-actions';
import { ReportBlockView } from './report-block';
import { openReportExport } from './report-export-dialog';
import { ReportStore } from './report.store';

/**
 * The incident report builder: reports of the project, their title, summary and sections, and the blocks
 * added from across the app ("Add to report"). Reorder with the arrows or Alt+↑/↓ on a focused block.
 */
@Component({
  selector: 'imt-report-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, MarkdownView, ReportBlockView],
  template: `
    <div class="scroll" role="region" aria-label="Report builder">
      <div class="bar">
        @if (reports.reports().length > 1) {
          <label class="pick">Report
            <select class="select" [value]="activeId()" (change)="reports.setActive($any($event.target).value)">
              @for (r of reports.reports(); track r.id) { <option [value]="r.id">{{ r.title || 'Untitled report' }}</option> }
            </select>
          </label>
        }
        <button type="button" class="btn" (click)="reports.create()"><imt-icon name="plus" [size]="13" /> New report</button>
        @if (reports.active(); as r) {
          <button type="button" class="btn" (click)="reports.remove(r.id)">Delete report</button>
          <span class="spacer"></span>
          <button type="button" class="btn primary" (click)="exportReport()">Export…</button>
        }
      </div>

      @if (reports.active(); as r) {
        <label class="field">Report title
          <input class="input title" [value]="r.title" (change)="reports.update(r.id, { title: val($event) })" />
        </label>
        <label class="field">Summary (Markdown)
          <textarea class="input summary" rows="3" [value]="r.summary" (change)="reports.update(r.id, { summary: val($event) })" placeholder="What happened, impact, cause…"></textarea>
        </label>
        @if (r.summary) { <imt-markdown-view class="md" [text]="r.summary" /> }

        <div class="add" role="toolbar" aria-label="Add to report">
          <span class="faint">Add:</span>
          <button type="button" class="btn" (click)="actions.addText()">Text</button>
          <button type="button" class="btn" [disabled]="!ws.selected()" (click)="actions.addSelectedEvent()">Selected event</button>
          <button type="button" class="btn" [disabled]="!bookmarks.items().length" (click)="actions.addBookmarks()">Bookmarks</button>
          <button type="button" class="btn" [disabled]="!ws.timeline()" (click)="actions.addTimeline()">Timeline</button>
          <button type="button" class="btn" [disabled]="!ws.summary()?.total" (click)="actions.addAnomalies()">Anomalies as findings</button>
          <button type="button" class="btn" [disabled]="!session.table()" (click)="actions.addQueryTable()">Query result</button>
          <button type="button" class="btn" [disabled]="!annotations.items().length" (click)="actions.addAnnotations()">Annotations</button>
        </div>
        <p class="faint hint">Charts: use “Add to report” on a chart in the Metrics view.</p>

        @for (s of r.sections; track s.id; let si = $index) {
          <section class="section" [attr.aria-label]="'Section ' + (s.title || si + 1)">
            <div class="section-head">
              <input class="input stitle" [attr.aria-label]="'Section title ' + (si + 1)" [value]="s.title" (change)="reports.renameSection(s.id, val($event))" />
              <button type="button" class="icon-btn" [attr.aria-label]="'Move section up: ' + s.title" [disabled]="si === 0" (click)="reports.moveSection(s.id, -1)"><span aria-hidden="true">↑</span></button>
              <button type="button" class="icon-btn" [attr.aria-label]="'Move section down: ' + s.title" [disabled]="si === r.sections.length - 1" (click)="reports.moveSection(s.id, 1)"><span aria-hidden="true">↓</span></button>
              <button type="button" class="icon-btn" [attr.aria-label]="'Remove section ' + s.title" (click)="reports.removeSection(s.id)"><imt-icon name="trash" [size]="13" /></button>
            </div>
            @for (b of s.blocks; track b.id; let bi = $index) {
              <imt-report-block [block]="b" [first]="si === 0 && bi === 0" [last]="si === r.sections.length - 1 && bi === s.blocks.length - 1" (moved)="refocus($event)" />
            } @empty {
              <p class="faint">Empty section. Blocks are added to the last section.</p>
            }
          </section>
        }
        <button type="button" class="btn add-section" (click)="reports.addSection()"><imt-icon name="plus" [size]="13" /> Add section</button>
      } @else {
        <div class="empty-next">
          <p class="faint">No report yet. Create one, or use “Add to report” on an event, a bookmark, a chart or the timeline.</p>
          <button type="button" class="btn primary" (click)="reports.create()"><imt-icon name="plus" [size]="13" /> Create a report</button>
        </div>
      }
    </div>
  `,
  styles: `
    .empty-next { display: flex; flex-direction: column; align-items: flex-start; gap: 8px; }
    :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .scroll { flex: 1; min-height: 0; overflow: auto; padding: 12px 16px; display: flex; flex-direction: column; gap: 10px; max-width: 900px; }
    .bar, .add { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
    .spacer { flex: 1; }
    .pick, .field { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--muted); }
    .pick { flex-direction: row; align-items: center; }
    .title { font-size: 15px; font-weight: 600; }
    .summary { resize: vertical; font-family: var(--font-mono); }
    .hint { margin: 0; font-size: 11px; }
    .section { display: flex; flex-direction: column; gap: 6px; padding-top: 6px; border-top: 1px solid var(--border); }
    .section-head { display: flex; gap: 4px; align-items: center; }
    .stitle { flex: 1; font-weight: 600; }
    .add-section { align-self: flex-start; }
    p { margin: 0; }
  `,
})
export class ReportView {
  protected readonly reports = inject(ReportStore);
  protected readonly activeId = computed(() => this.reports.active()?.id ?? '');
  protected readonly actions = inject(ReportActions);
  protected readonly ws = inject(WorkspaceStore);
  protected readonly bookmarks = inject(BookmarkStore);
  protected readonly session = inject(QuerySession);
  protected readonly annotations = inject(TimelineAnnotations);
  private readonly dialogs = inject(DialogService);
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);

  protected exportReport(): void {
    void openReportExport(this.dialogs);
  }

  /** Keeps the keyboard on a block after it moved (moving a node out of the DOM drops its focus). */
  protected refocus(id: string): void {
    setTimeout(() => this.host.nativeElement.querySelector<HTMLElement>(`[data-block-id="${CSS.escape(id)}"]`)?.focus());
  }

  protected val = (e: Event) => (e.target as HTMLInputElement | HTMLTextAreaElement).value;
}
