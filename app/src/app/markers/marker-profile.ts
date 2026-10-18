import { inject, Injectable } from '@angular/core';
import { WorkspaceSummary } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { ProfileModel } from '../core/profile.model';
import { WorkspaceStore } from '../core/workspace.store';
import { ProfileUndo } from '../undo/profile-undo';
import { parseProfile, stringifyProfile } from '../profile/toml';
import { RuleRow, SaveFailure } from './markers.types';
import { readRows } from './marker-rules';

export type CommitResult = { ok: true; summary: WorkspaceSummary } | { ok: false; failure: SaveFailure };

/**
 * Loads the active profile, applies a change to a copy, validates it in the engine and saves it
 * (which re-imports). Failures carry the engine's message and the line in the generated TOML.
 */
@Injectable({ providedIn: 'root' })
export class MarkerProfile {
  private readonly backend = inject(ImtBackend);
  private readonly store = inject(WorkspaceStore);
  private readonly undo = inject(ProfileUndo);

  /** The marker rules (active and disabled) in priority order, and the model they came from. */
  async load(): Promise<{ model: ProfileModel; rows: RuleRow[] }> {
    const doc = await this.backend.getProfile();
    const model = parseProfile(doc.toml);
    return { model, rows: readRows(model) };
  }

  /** Applies `mutate` to a fresh copy of the profile and saves it (undoable as `label`). Never throws. */
  async commit(mutate: (model: ProfileModel, rows: RuleRow[]) => void, label = $localize`:Undo history entry|@@markers.profile.undoLabel:marker rule edit`): Promise<CommitResult> {
    try {
      const before = (await this.backend.getProfile()).toml;
      const model = parseProfile(before);
      const rows = readRows(model);
      mutate(model, rows);
      const toml = stringifyProfile(model);
      const validation = await this.backend.validateProfile(toml);
      if (!validation.ok) {
        const line = validation.line ?? null;
        const excerpt = line ? (toml.split('\n')[line - 1]?.trim().slice(0, 160) ?? null) : null;
        return { ok: false, failure: { message: validation.error ?? $localize`:@@markers.profile.invalid:The profile is not valid.`, line, excerpt } };
      }
      const summary = await this.backend.saveProfile(toml);
      this.store.profileSaved(summary);
      this.undo.record(label, before, toml);
      return { ok: true, summary };
    } catch (e) {
      return { ok: false, failure: { message: e instanceof Error ? e.message : String(e), line: null, excerpt: null } };
    }
  }
}
