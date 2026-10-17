import { inject, Injectable } from '@angular/core';
import { WorkspaceSummary } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { ProfileModel } from '../core/profile.model';
import { WorkspaceStore } from '../core/workspace.store';
import { ProfileUndo } from '../undo/profile-undo';
import { parseProfile, stringifyProfile } from './toml';

/**
 * Applies a small change to the active profile without opening the profile editor: loads the
 * current TOML, lets `mutate` change a copy, validates, saves (which re-imports) and announces it.
 * Used by the "from data" dialogs (drop events like this, extract a value, mark events, ...).
 */
@Injectable({ providedIn: 'root' })
export class ProfileEdits {
  private readonly backend = inject(ImtBackend);
  private readonly store = inject(WorkspaceStore);
  private readonly undo = inject(ProfileUndo);

  /** The current profile as an editable model (a fresh copy every call). */
  async current(): Promise<{ toml: string; model: ProfileModel }> {
    const doc = await this.backend.getProfile();
    return { toml: doc.toml, model: parseProfile(doc.toml) };
  }

  /** Serialises a model the way the profile editor does (comments are not kept). */
  toToml(model: ProfileModel): string {
    return stringifyProfile(model);
  }

  /** Saves and applies the change (undoable as `label`). Rejects with the backend's message when the profile is invalid. */
  async apply(mutate: (model: ProfileModel) => void, label = 'profile change'): Promise<WorkspaceSummary> {
    const { toml: before, model } = await this.current();
    mutate(model);
    const toml = stringifyProfile(model);
    const validation = await this.backend.validateProfile(toml);
    if (!validation.ok) throw new Error(validation.error ?? 'The profile is not valid.');
    const summary = await this.backend.saveProfile(toml);
    this.store.profileSaved(summary);
    this.undo.record(label, before, toml);
    return summary;
  }
}
