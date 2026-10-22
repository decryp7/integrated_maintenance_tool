import { ChangeDetectionStrategy, Component, inject, OnInit } from '@angular/core';
import { Playbook } from '../../core/api.types';
import { Platform } from '../../core/platform';
import { Icon } from '../../shared/icon';
import { actionLabel, progressOf, progressText } from './playbook-model';
import { PlaybookEditor } from './playbook-editor';
import { PlaybookRunView } from './playbook-run';
import { PlaybookStore } from './playbook.store';

const JSON_FILES = [{ name: 'JSON', extensions: ['json'] }];

/**
 * The Playbooks pane: saved investigation runbooks. The list holds your playbooks and the templates; a playbook can be
 * started as a run whose steps run against the open data and are recorded in the project, or edited.
 */
@Component({
  selector: 'imt-playbooks-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, PlaybookEditor, PlaybookRunView],
  template: `
    <div class="bar">
      <button type="button" class="btn" data-testid="pb-new" (click)="store.startNew()"><imt-icon name="plus" [size]="12" /> <span i18n="@@playbooks.new">New playbook</span></button>
      <button type="button" class="btn" data-testid="pb-import" (click)="importFile()" i18n="@@playbooks.import">Import</button>
      <span class="spacer"></span>
      @if (store.loading()) {
        <span class="faint" i18n="@@playbooks.loading">Loading...</span>
      }
    </div>
    @if (store.loadError(); as error) {
      <div class="error" role="alert" data-testid="pb-load-error">
        <span i18n="@@playbooks.loadError">The playbooks could not be listed: {{ error }}</span>
        <button type="button" class="btn" (click)="store.load()" i18n="@@playbooks.retry">Retry</button>
      </div>
    }
    @if (store.notice(); as notice) {
      <div class="notice" role="status" data-testid="pb-notice">
        {{ notice }}
        <button type="button" class="icon-btn" aria-label="Dismiss" i18n-aria-label="@@playbooks.dismissNotice" (click)="store.dismissNotice()"><imt-icon name="close" [size]="12" /></button>
      </div>
    }
    @for (w of warnings(); track $index) {
      <div class="warning" role="status" data-testid="pb-warning">{{ w }}</div>
    }
    <div class="body">
      <nav class="list" aria-label="Playbooks" i18n-aria-label="@@playbooks.list">
        <h3 i18n="@@playbooks.mine">My playbooks</h3>
        @for (p of store.library().playbooks; track p.id) {
          <button type="button" class="item" [class.sel]="p.id === store.selectedId()" [attr.aria-current]="p.id === store.selectedId() ? 'true' : null" [attr.data-testid]="'pb-item-' + p.id" (click)="store.select(p.id)">{{ p.title }}</button>
        } @empty {
          <p class="faint" data-testid="pb-mine-empty" i18n="@@playbooks.mineEmpty">None yet: create one or use a template.</p>
        }
        <h3 i18n="@@playbooks.templates">Templates</h3>
        @for (p of store.library().templates; track p.id) {
          <button type="button" class="item" [class.sel]="p.id === store.selectedId()" [attr.aria-current]="p.id === store.selectedId() ? 'true' : null" [attr.data-testid]="'pb-template-' + p.id" (click)="store.select(p.id)">{{ p.title }}</button>
        }
      </nav>
      <section class="details" data-testid="pb-details" aria-live="polite">
        @if (store.editing()) {
          <imt-playbook-editor />
        } @else if (store.selected(); as pb) {
          <h2 class="title">{{ pb.title }}</h2>
          @if (pb.description) {
            <p class="description">{{ pb.description }}</p>
          }
          <div class="buttons">
            @if (store.selectedIsTemplate()) {
              <button type="button" class="btn primary" data-testid="pb-use-template" (click)="store.saveCopyOfTemplate(pb)" i18n="@@playbooks.useTemplate">Use this template</button>
            } @else {
              <button type="button" class="btn primary" data-testid="pb-start" (click)="store.startRun(pb)" i18n="@@playbooks.start">Start run</button>
              <button type="button" class="btn" data-testid="pb-edit" (click)="store.startEdit(pb)" i18n="@@playbooks.edit">Edit</button>
              <button type="button" class="btn" data-testid="pb-export" (click)="exportFile(pb)" i18n="@@playbooks.export">Export</button>
              <button type="button" class="btn" data-testid="pb-delete" (click)="store.remove(pb.id)" i18n="@@playbooks.delete">Delete</button>
            }
          </div>
          @if (store.actionError(); as e) {
            <div class="error" role="alert" data-testid="pb-action-error">
              <span>{{ e }}</span>
              <button type="button" class="icon-btn" aria-label="Dismiss" i18n-aria-label="@@playbooks.dismissError" (click)="store.dismissError()"><imt-icon name="close" [size]="12" /></button>
            </div>
          }
          @if (store.activeRun(); as run) {
            <h3 i18n="@@playbooks.runHeading">Run</h3>
            <imt-playbook-run [run]="run" />
          } @else {
            <ol class="outline" data-testid="pb-outline">
              @for (s of pb.steps; track s.id) {
                <li>
                  <strong>{{ s.title }}</strong> <span class="faint">{{ kind(s.action.type) }}</span>
                </li>
              }
            </ol>
            @if (store.selectedRuns().length) {
              <h3 i18n="@@playbooks.pastRuns">Past runs</h3>
              <ul class="runs" data-testid="pb-runs">
                @for (r of store.selectedRuns(); track r.id) {
                  <li>
                    <span>{{ started(r.startedAt) }} - {{ progressLabel(r) }}</span>
                    <button type="button" class="btn" [attr.data-testid]="'pb-open-run-' + r.id" (click)="store.openRun(r.id)" i18n="@@playbooks.openRun">Open</button>
                    <button type="button" class="btn" [attr.data-testid]="'pb-delete-run-' + r.id" (click)="store.deleteRun(r.id)" i18n="@@playbooks.deleteRun">Delete</button>
                  </li>
                }
              </ul>
            }
          }
        } @else {
          <p class="faint" data-testid="pb-none" i18n="@@playbooks.none">Select a playbook, or start from a template.</p>
        }
      </section>
    </div>
  `,
  styles: `
    :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); container-type: inline-size; }
    .bar { flex: none; display: flex; align-items: center; gap: 8px; padding: 4px 12px; border-bottom: 1px solid var(--border); font-size: var(--fs-sm); }
    .spacer { flex: 1; }
    .error, .notice, .warning { flex: none; display: flex; align-items: center; gap: 8px; padding: 4px 12px; font-size: var(--fs-sm); border-bottom: 1px solid var(--border); }
    .error { color: var(--danger-text); }
    .warning { color: var(--warn-text); }
    .error span { flex: 1; }
    .body { flex: 1; min-height: 0; display: grid; grid-template-columns: minmax(180px, 30%) 1fr; }
    @container (max-width: 560px) { .body { grid-template-columns: 1fr; grid-template-rows: auto 1fr; } .details { border-left: 0; border-top: 1px solid var(--border); } }
    .list { min-height: 0; overflow: auto; padding: 4px 0; display: flex; flex-direction: column; }
    .list h3 { margin: 8px 12px 2px; font-size: var(--fs-xs); color: var(--muted); text-transform: uppercase; }
    .list p { margin: 2px 12px; }
    .item { text-align: left; border: 0; background: transparent; color: var(--text-2); padding: 4px 12px; min-height: 24px; cursor: pointer; font-size: var(--fs-sm); }
    .item:hover { background: var(--bg-hover); }
    .item.sel { background: var(--bg-sel); color: var(--text); }
    .item:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
    .details { min-height: 0; overflow: auto; padding: 8px 14px; border-left: 1px solid var(--border); font-size: var(--fs-sm); display: flex; flex-direction: column; gap: 8px; }
    .details h2, .details h3 { margin: 0; }
    .details h2 { font-size: var(--fs-md); }
    .description { margin: 0; color: var(--text-2); }
    .buttons { display: flex; flex-wrap: wrap; gap: 6px; }
    .outline, .runs { margin: 0; padding-left: 20px; }
    .runs { list-style: none; padding: 0; display: flex; flex-direction: column; gap: 4px; }
    .runs li { display: flex; align-items: center; gap: 8px; }
    .faint { color: var(--muted); }
  `,
})
export class PlaybooksPanel implements OnInit {
  protected readonly store = inject(PlaybookStore);
  private readonly platform = inject(Platform);
  protected readonly kind = actionLabel;

  protected warnings(): string[] {
    return [...this.store.library().warnings, ...this.store.restoreWarnings()];
  }
  protected progressLabel(run: Parameters<typeof progressOf>[0]): string {
    return progressText(progressOf(run));
  }
  protected started(at: number): string {
    return new Date(at).toLocaleString();
  }

  ngOnInit(): void {
    if (!this.store.loaded()) void this.store.load();
  }

  protected async importFile(): Promise<void> {
    const path = await this.platform.pickOpenPath($localize`:@@playbooks.importTitle:Import a playbook`, JSON_FILES);
    if (path) await this.store.import(path);
  }

  protected async exportFile(pb: Playbook): Promise<void> {
    const path = await this.platform.pickSavePath($localize`:@@playbooks.exportTitle:Export playbook`, `${pb.id}.json`, JSON_FILES);
    if (path) await this.store.export(pb, path);
  }
}
