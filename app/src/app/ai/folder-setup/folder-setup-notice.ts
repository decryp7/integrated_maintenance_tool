import { effect, inject, Injectable, untracked } from '@angular/core';
import { WorkspaceStore } from '../../core/workspace.store';
import { NotificationsStore } from '../../notifications/notifications.store';
import { FolderSetupLauncher } from './provide-folder-setup';

/** Opened files from this many sources look like a folder worth setting up. */
export const FOLDER_SOURCES = 2;
/** Key of the message in the notification centre (a new open replaces it). */
export const FOLDER_SETUP_NOTICE = 'folder-setup';

/**
 * Right after the user opens a folder of several files (not on start-up or restores), offers "Set up analysis…" as a
 * message in the notification centre (the bell), not as a banner over the workspace.
 */
@Injectable({ providedIn: 'root' })
export class FolderSetupNotice {
  private readonly store = inject(WorkspaceStore);
  private readonly notices = inject(NotificationsStore);
  private readonly launcher = inject(FolderSetupLauncher);

  constructor() {
    let seen = this.store.importVersion();
    effect(() => {
      const version = this.store.importVersion();
      if (version === seen) return;
      seen = version;
      untracked(() => this.offer());
    });
  }

  private offer(): void {
    const summary = this.store.summary();
    if (!summary || summary.sources.length + summary.unrecognized.length < FOLDER_SOURCES) return;
    this.notices.postLocal({
      key: FOLDER_SETUP_NOTICE,
      source: 'workspace',
      severity: 'info',
      title: $localize`:@@ai.folderSetup.notice.title:New folder? Set up analysis`,
      body: $localize`:@@ai.folderSetup.hint.text:New folder? IMT can survey it and propose lanes, markers, extractors and charts for you to review.`,
      actionLabel: $localize`:@@ai.folderSetup.hint.open:Set up analysis…`,
      run: () => this.launcher.open(),
    });
  }
}
