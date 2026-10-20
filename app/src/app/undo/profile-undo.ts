import { inject, Injectable } from '@angular/core';
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
  async save(label: string, toml: string): Promise<number> {
    const before = (await this.backend.getProfile()).toml;
    const job = await this.backend.saveProfile(toml);
    void this.store.applyProfile(job);
    this.record(label, before, toml);
    return job;
  }

  private async apply(toml: string): Promise<void> {
    void this.store.applyProfile(await this.backend.saveProfile(toml));
  }
}
