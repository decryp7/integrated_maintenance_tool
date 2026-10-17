import { inject, Injectable, Injector, signal } from '@angular/core';
import { BundleManifest } from '../../core/api.types';
import { Platform } from '../../core/platform';
import { WorkspaceStore } from '../../core/workspace.store';
import { UiStateRegistry } from '../../lifecycle/ui-state';
import { ProjectService } from '../../projects/project.service';
import { baseName, errorMessage } from '../../tools/tool-errors';
import { DialogService } from '../../ui/dialog.service';
import { ReportStore } from '../report/report.store';
import { ShareBackend } from '../share-backend';
import { BundleDialog } from './bundle-dialog';

export const BUNDLE_EXTENSION = '.imtz';
export const BUNDLE_FILTERS = [{ name: 'IMT bundle', extensions: ['imtz'] }];

export function isBundlePath(path: string): boolean {
  return path.toLowerCase().endsWith(BUNDLE_EXTENSION);
}

/** The bundle the window shows (read-only), for the banner. */
export interface OpenedBundle {
  path: string;
  name: string;
  manifest: BundleManifest | null;
}

/** Shareable `.imtz` bundles: create (dialog), open read-only, and leave. */
@Injectable({ providedIn: 'root' })
export class BundleService {
  private readonly injector = inject(Injector);
  /** Resolved on use, so hosts that never share (e.g. the banners in tests) need no share backend. */
  private get share(): ShareBackend {
    return this.injector.get(ShareBackend);
  }
  private readonly store = inject(WorkspaceStore);
  private readonly registry = inject(UiStateRegistry);
  private readonly reports = inject(ReportStore);
  private readonly projects = inject(ProjectService);
  private readonly platform = inject(Platform);
  private readonly dialogs = inject(DialogService);

  /** The open bundle; null when the window shows a normal project. */
  readonly opened = signal<OpenedBundle | null>(null);
  readonly busy = signal(false);

  async create(): Promise<void> {
    await this.dialogs.open(BundleDialog, { title: 'Share as bundle', width: 560 }).closed;
  }

  async openFromDialog(): Promise<void> {
    const path = await this.platform.pickOpenPath('Open bundle', BUNDLE_FILTERS);
    if (path) await this.open(path);
  }

  /** Opens a bundle read-only: its profile, files, UI state and reports; saving asks for a new project file. */
  async open(path: string): Promise<boolean> {
    this.busy.set(true);
    try {
      const o = await this.share.openBundle(path);
      this.store.profileSaved(o.summary);
      this.registry.restore(o.ui);
      this.reports.replace(o.reports);
      this.projects.adoptReadOnly(o.name || baseName(path).replace(/\.imtz$/i, ''), o.missing);
      this.opened.set({ path, name: o.name || baseName(path), manifest: o.bundle });
      this.store.setNotice(`Opened ${baseName(path)} (read-only)`);
      return true;
    } catch (e) {
      this.store.setError(`Could not open the bundle: ${errorMessage(e)}`);
      return false;
    } finally {
      this.busy.set(false);
    }
  }

  /** Saves what the bundle shows as a project of its own. */
  async saveAsProject(): Promise<void> {
    if (await this.projects.saveAs()) this.opened.set(null);
  }

  /** Leaves the bundle: an empty, untitled project. */
  async close(): Promise<void> {
    this.opened.set(null);
    await this.projects.newProject();
  }
}
