import { inject, Injectable } from '@angular/core';
import { WorkspaceSummary } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { WorkspaceStore } from '../core/workspace.store';
import { UndoStack } from './undo-stack';

/**
 * Undo for edits that change the active profile (marker rules, the profile editor, dashboards, live rules,
 * "from data" dialogs, history restores, templates). Undo and redo save the profile text from before or
 * after the edit through the engine's normal save, which validates it first, and re-import.
 */
@Injectable({ providedIn: 'root' })
export class ProfileUndo {
  private readonly backend = inject(ImtBackend);
  private readonly store = inject(WorkspaceStore);
  private readonly undo = inject(UndoStack);

  /** Records a profile change that has been saved: `before` and `after` are the whole profile texts. */
  record(label: string, before: string, after: string): void {
    if (before === after) return;
    this.undo.record({ label, do: () => this.apply(after), undo: () => this.apply(before) });
  }

  /**
   * Saves `toml` as the active profile (the engine validates it and re-imports), announces it and records
   * the change. Rejects with the engine's message when the profile is refused; nothing is recorded then.
   */
  async save(label: string, toml: string): Promise<WorkspaceSummary> {
    const before = (await this.backend.getProfile()).toml;
    const summary = await this.backend.saveProfile(toml);
    this.store.profileSaved(summary);
    this.record(label, before, toml);
    return summary;
  }

  private async apply(toml: string): Promise<void> {
    this.store.profileSaved(await this.backend.saveProfile(toml));
  }
}
