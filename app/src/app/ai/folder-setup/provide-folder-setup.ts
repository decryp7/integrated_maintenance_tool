import { EnvironmentProviders, inject, Injectable, makeEnvironmentProviders } from '@angular/core';
import { WorkspaceStore } from '../../core/workspace.store';
import { IMT_ACTIVITY_ITEMS, IMT_COMMANDS, ImtActivityItem, ImtCommand } from '../../extensions/tokens';
import { DialogService } from '../../ui/dialog.service';
import { FolderSetupStore } from './folder-setup.store';
import { FolderSetupWizard } from './folder-setup-wizard';

export const FOLDER_SETUP_COMMAND = 'folder.setupAnalysis';

/** Opens the wizard and starts a survey; closing it stops a survey or AI refinement still running. */
@Injectable({ providedIn: 'root' })
export class FolderSetupLauncher {
  private readonly dialogs = inject(DialogService);
  private readonly store = inject(FolderSetupStore);

  open(): void {
    void this.store.start();
    const ref = this.dialogs.open(FolderSetupWizard, {
      title: $localize`:Dialog title@@ai.folderSetup.title:Set up analysis for this folder`,
      width: 820,
    });
    void ref.closed.then(() => {
      if (this.store.phase() === 'surveying') void this.store.cancel();
      if (this.store.ai() === 'running') void this.store.cancelAi();
    });
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
