import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { WorkspaceStore } from '../../core/workspace.store';
import { Icon } from '../../shared/icon';
import { FolderSetupLauncher } from './provide-folder-setup';

/** Opened files from this many sources look like a folder worth setting up. */
export const FOLDER_SOURCES = 2;

/**
 * A banner offered right after the user opens a folder of several files (not on start-up or restores):
 * "Set up analysis for this folder…". Dismissed until the next open.
 */
@Component({
  selector: 'imt-folder-setup-hint',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    @if (visible()) {
      <aside class="hint" role="region" aria-label="Folder set-up" i18n-aria-label="@@ai.folderSetup.hint.label" data-testid="folder-setup-hint">
        <imt-icon name="timeline" [size]="15" />
        <span i18n="@@ai.folderSetup.hint.text">New folder? IMT can survey it and propose lanes, markers, extractors and charts for you to review.</span>
        <button type="button" class="btn primary" data-testid="folder-setup-hint-open" (click)="open()" i18n="@@ai.folderSetup.hint.open">Set up analysis…</button>
        <button type="button" class="icon-btn" aria-label="Dismiss" title="Dismiss" i18n-aria-label="@@ai.folderSetup.hint.dismiss" i18n-title="@@ai.folderSetup.hint.dismissTitle" (click)="dismiss()"><imt-icon name="close" [size]="14" /></button>
      </aside>
    }
  `,
  styles: `
    .hint { display: flex; align-items: center; gap: 10px; padding: 6px 12px; background: var(--accent-soft); border-bottom: 1px solid var(--border); font-size: 12px; color: var(--text); }
    .hint span { flex: 1; min-width: 0; }
  `,
})
export class FolderSetupHint {
  private readonly store = inject(WorkspaceStore);
  private readonly launcher = inject(FolderSetupLauncher);
  /** The import version the banner was dismissed (or used) at; it comes back on the next open. */
  private readonly handledAt = signal(this.store.importVersion());

  protected readonly visible = computed(() => {
    const version = this.store.importVersion();
    const summary = this.store.summary();
    return version > this.handledAt() && !!summary && summary.sources.length + summary.unrecognized.length >= FOLDER_SOURCES;
  });

  protected open(): void {
    this.handledAt.set(this.store.importVersion());
    this.launcher.open();
  }

  protected dismiss(): void {
    this.handledAt.set(this.store.importVersion());
  }
}
