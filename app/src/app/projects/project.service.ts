import { computed, effect, inject, Injectable, signal } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { ProjectBackend } from '../core/tools-backend';
import { UiStateRegistry } from '../lifecycle/ui-state';
import { baseName, errorMessage } from '../tools/tool-errors';
import { ProjectPicker } from './project-picker';
import { RECENT_PROJECTS } from './recent-projects';
import { ReportStore } from '../share/report/report.store';
import { OverwriteGuard } from '../ui/overwrite-guard';
import { plural } from '../i18n/locale';

export const PROJECT_EXTENSION = '.imtproj';
export const UNTITLED = $localize`:Name of a project that was not saved yet|@@projects.untitled:Untitled`;

export function isProjectPath(path: string): boolean {
  return path.toLowerCase().endsWith(PROJECT_EXTENSION);
}

/** Project name suggested for a file: its name without the extension. */
export function projectNameFor(path: string): string {
  return baseName(path).replace(/\.imtproj$/i, '') || UNTITLED;
}

/**
 * The current project: its file, name and unsaved state, and New / Open / Save / Save As.
 * What is saved is the engine's workspace (sources, profile) plus the serialized UI state.
 */
@Injectable({ providedIn: 'root' })
export class ProjectService {
  private readonly store = inject(WorkspaceStore);
  private readonly backend = inject(ProjectBackend);
  private readonly registry = inject(UiStateRegistry);
  private readonly picker = inject(ProjectPicker);
  private readonly recentStore = inject(RECENT_PROJECTS);
  private readonly reports = inject(ReportStore);
  private readonly overwrite = inject(OverwriteGuard);

  readonly path = signal<string | null>(null);
  readonly name = signal(UNTITLED);
  readonly busy = signal(false);
  /** Project paths that no longer exist, from the last open (shown as a notice). */
  readonly missing = signal<string[]>([]);
  readonly recent = computed(() => this.recentStore.list());

  /** Identity of what a project would save; compared with the key at the last save/open. */
  private readonly key = computed(() => `${(this.store.summary()?.sources ?? []).map((s) => s.path).join('|')}\n${this.registry.trackedSnapshot()}\n${this.reports.json()}`);
  private readonly baseline = signal<string | null>(null);

  readonly dirty = computed(() => this.baseline() !== null && this.baseline() !== this.key());
  /** Window/tab title: "● name — IMT" while unsaved. */
  readonly title = computed(() => `${this.dirty() ? '● ' : ''}${this.name()} — IMT`);

  constructor() {
    // The first loaded workspace is the clean starting point.
    effect(() => {
      if (this.baseline() === null && this.store.summary()) this.baseline.set(this.key());
    });
    effect(() => {
      document.title = this.title();
    });
  }

  /** Starts an empty, untitled project (closes the workspace and resets the UI state). */
  async newProject(): Promise<void> {
    await this.store.clearWorkspace();
    this.registry.restore(null);
    this.reports.replace([]);
    this.path.set(null);
    this.name.set(UNTITLED);
    this.missing.set([]);
    this.markClean();
  }

  async openFromDialog(): Promise<void> {
    const path = await this.picker.pickOpen();
    if (path) await this.open(path);
  }

  /**
   * Opens a project file. With `session` the file is the autosaved session: its content is restored
   * but it does not become the current project and is not added to the recent list.
   */
  async open(path: string, options: { session?: boolean } = {}): Promise<boolean> {
    this.busy.set(true);
    try {
      const opened = await this.backend.openProject(path);
      this.store.profileSaved(opened.summary);
      this.registry.restore(opened.ui);
      this.reports.replace(opened.reports ?? []);
      this.missing.set(opened.missing);
      if (!options.session) {
        this.path.set(opened.path);
        this.name.set(opened.name);
        this.remember(opened.path, opened.name);
      } else {
        this.path.set(null);
        this.name.set(UNTITLED);
      }
      this.markClean();
      if (opened.missing.length) {
        const n = opened.missing.length;
        this.store.setNotice(
          plural(n, {
            one: $localize`:@@projects.service.missing.one:${n}:count: path of this project could not be found`,
            other: $localize`:@@projects.service.missing.other:${n}:count: paths of this project could not be found`,
          }),
        );
      }
      return true;
    } catch (e) {
      this.store.setError($localize`:@@projects.service.openFailed:Could not open the project: ${errorMessage(e)}:error:`);
      return false;
    } finally {
      this.busy.set(false);
    }
  }

  /** Saves to the project's file; asks for one when the project is new. */
  async save(): Promise<boolean> {
    const path = this.path();
    // The project's own file: saving over it is the point of Save.
    return path ? this.write(path, this.name(), 'own-file') : this.saveAs();
  }

  async saveAs(): Promise<boolean> {
    const path = await this.picker.pickSave(`${this.name()}${PROJECT_EXTENSION}`);
    return path ? this.write(path, projectNameFor(path), 'save-dialog') : false;
  }

  /**
   * Shows what was just opened from a read-only source (a shared bundle) as an unsaved project with
   * this name: there is no project file, so Save asks where to save it.
   */
  adoptReadOnly(name: string, missing: string[] = []): void {
    this.path.set(null);
    this.name.set(name || UNTITLED);
    this.missing.set(missing);
    this.markClean();
  }

  dismissMissing(): void {
    this.missing.set([]);
  }

  clearRecent(): void {
    this.recentStore.clear();
  }

  private async write(path: string, name: string, origin: 'own-file' | 'save-dialog'): Promise<boolean> {
    this.busy.set(true);
    try {
      // What is written is what this key describes: edits made while the save is in flight stay unsaved.
      let written = this.key();
      const save = (overwrite: boolean) => {
        written = this.key();
        return this.backend.saveProject(path, name, this.registry.serialize(), this.reports.reports(), overwrite);
      };
      const saved = origin === 'own-file' ? await save(true) : await this.overwrite.write(path, save);
      if (!saved) return false;
      this.path.set(saved.path);
      this.name.set(saved.name);
      this.remember(saved.path, saved.name);
      this.baseline.set(written);
      this.store.setNotice($localize`:@@projects.service.saved:Saved ${saved.name}:name:`);
      return true;
    } catch (e) {
      this.store.setError($localize`:@@projects.service.saveFailed:Could not save the project: ${errorMessage(e)}:error:`);
      return false;
    } finally {
      this.busy.set(false);
    }
  }

  private remember(path: string, name: string): void {
    this.recentStore.add({ path, name });
  }

  private markClean(): void {
    this.baseline.set(this.key());
  }
}
