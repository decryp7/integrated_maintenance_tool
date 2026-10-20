import { EnvironmentProviders, inject, Injectable, makeEnvironmentProviders, provideEnvironmentInitializer } from '@angular/core';
import { FolderSetupNotice } from './folder-setup-notice';
import { WorkspaceStore } from '../../core/workspace.store';
import { IMT_ACTIVITY_ITEMS, IMT_COMMANDS, ImtActivityItem, ImtCommand } from '../../extensions/tokens';
import { DialogService } from '../../ui/dialog.service';
import { FolderSetupStore } from './folder-setup.store';
import { FolderSetupWizard } from './folder-setup-wizard';

export const FOLDER_SETUP_COMMAND = 'folder.setupAnalysis';

/**
 * Opens the wizard and starts a survey. Closing it leaves a running survey or AI refinement going in Jobs: its
 * message's View reopens the wizard with the result (`show`).
 */
@Injectable({ providedIn: 'root' })
export class FolderSetupLauncher {
  private readonly dialogs = inject(DialogService);
  private readonly store = inject(FolderSetupStore);

  open(): void {
    void this.store.start();
    this.show();
  }

  /** Opens the wizard on what the store holds (View of a finished survey or refinement). */
  show(): void {
    const ref = this.dialogs.open(FolderSetupWizard, {
      title: $localize`:Dialog title@@ai.folderSetup.title:Set up analysis for this folder`,
      width: 820,
    });
    void ref.closed.then(() => this.store.detach());
  }
}

/** Something is open to survey (events, or files no importer read yet). */
export function hasOpenFiles(store: InstanceType<typeof WorkspaceStore>): boolean {
  const s = store.summary();
  return !!s && (s.sources.length > 0 || s.unrecognized.length > 0);
}

export function folderSetupCommands(launcher: FolderSetupLauncher, store: InstanceType<typeof WorkspaceStore>): ImtCommand[] {
  return [
    {
      id: FOLDER_SETUP_COMMAND,
      title: $localize`:Command and File menu item@@ai.folderSetup.cmd:Set up analysis for this folder…`,
      category: $localize`:Command palette category@@ai.folderSetup.category:File`,
      when: () => hasOpenFiles(store),
      run: () => launcher.open(),
    },
  ];
}

/** One line in `app.config.ts`: the command (palette, File menu) and its activity-bar button. The hint banner sits in the app shell. */
export function provideFolderSetup(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: IMT_COMMANDS, multi: true, useFactory: () => folderSetupCommands(inject(FolderSetupLauncher), inject(WorkspaceStore)) },
    { provide: IMT_ACTIVITY_ITEMS, multi: true, useFactory: () => folderSetupActivityItems(inject(WorkspaceStore)) },
    // After a folder is opened: a message in the notification centre offers the set-up.
    provideEnvironmentInitializer(() => inject(FolderSetupNotice)),
  ]);
}

/** The activity-bar button: the same command as File ▸ Set up analysis for this folder…, while files are open. */
export function folderSetupActivityItems(store: InstanceType<typeof WorkspaceStore>): ImtActivityItem[] {
  return [
    {
      id: 'folder-setup',
      label: $localize`:Activity bar button@@ai.folderSetup.activity:Set up analysis for this folder`,
      icon: 'analysis',
      order: 950,
      when: () => hasOpenFiles(store),
      run: { command: FOLDER_SETUP_COMMAND },
    },
  ];
}
