import { computed, effect, inject, Injectable, signal } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { ProjectBackend } from '../core/tools-backend';
import { UiStateRegistry } from '../lifecycle/ui-state';
import { baseName, errorMessage } from '../tools/tool-errors';
import { ProjectPicker } from './project-picker';
import { RECENT_PROJECTS } from './recent-projects';

export const PROJECT_EXTENSION = '.imtproj';
export const UNTITLED = 'Untitled';

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

  readonly path = signal<string | null>(null);
  readonly name = signal(UNTITLED);
  readonly busy = signal(false);
  /** Project paths that no longer exist, from the last open (shown as a notice). */
  readonly missing = signal<string[]>([]);
  readonly recent = signal(this.recentStore.list());

  /** Identity of what a project would save; compared with the key at the last save/open. */
  private readonly key = computed(() => `${(this.store.summary()?.sources ?? []).map((s) => s.path).join('|')}\n${this.registry.trackedSnapshot()}`);
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
      if (opened.missing.length) this.store.setNotice(`${opened.missing.length} path${opened.missing.length === 1 ? '' : 's'} of this project could not be found`);
      return true;
    } catch (e) {
      this.store.setError(`Could not open the project: ${errorMessage(e)}`);
      return false;
    } finally {
      this.busy.set(false);
    }
  }

  /** Saves to the project's file; asks for one when the project is new. */
  async save(): Promise<boolean> {
    const path = this.path();
    return path ? this.write(path, this.name()) : this.saveAs();
  }

  async saveAs(): Promise<boolean> {
    const path = await this.picker.pickSave(`${this.name()}${PROJECT_EXTENSION}`);
    return path ? this.write(path, projectNameFor(path)) : false;
  }

  dismissMissing(): void {
    this.missing.set([]);
  }

  clearRecent(): void {
    this.recentStore.clear();
    this.recent.set([]);
  }

  private async write(path: string, name: string): Promise<boolean> {
    this.busy.set(true);
    try {
      const saved = await this.backend.saveProject(path, name, this.registry.serialize());
      this.path.set(saved.path);
      this.name.set(saved.name);
      this.remember(saved.path, saved.name);
      this.markClean();
      this.store.setNotice(`Saved ${saved.name}`);
      return true;
    } catch (e) {
      this.store.setError(`Could not save the project: ${errorMessage(e)}`);
      return false;
    } finally {
      this.busy.set(false);
    }
  }

  private remember(path: string, name: string): void {
    this.recentStore.add({ path, name });
    this.recent.set(this.recentStore.list());
  }

  private markClean(): void {
    this.baseline.set(this.key());
  }
}
