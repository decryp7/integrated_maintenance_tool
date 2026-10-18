import { inject, Injectable } from '@angular/core';
import { ImtBackend } from '../core/backend';
import { describeError } from '../core/logger';
import { WorkspaceStore } from '../core/workspace.store';
import { ProfileEdits } from '../profile/profile-edits';
import { ProfileUndo } from '../undo/profile-undo';
import { LiveSettingsDraft, readLive, writeLive } from './live-rules.model';

/** An engine validation failure, with the line of the generated TOML it points at. */
export interface RulesFailure {
  message: string;
  line: number | null;
  excerpt: string | null;
}

export type RulesResult = { ok: true } | { ok: false; failure: RulesFailure };

/**
 * Loads and saves the `[live]` section through the profile flow: the change is applied to a fresh copy of the
 * active profile, validated by the engine, and only then saved (which applies it).
 */
@Injectable({ providedIn: 'root' })
export class LiveRulesProfile {
  private readonly edits = inject(ProfileEdits);
  private readonly backend = inject(ImtBackend);
  private readonly store = inject(WorkspaceStore);
  private readonly undo = inject(ProfileUndo);

  async load(): Promise<LiveSettingsDraft> {
    const { model } = await this.edits.current();
    return readLive(model);
  }

  /** Validates the draft in the engine; never throws. */
  validate(draft: LiveSettingsDraft): Promise<RulesResult> {
    return this.run(draft, false);
  }

  /** Validates, then saves and applies the draft; never throws. */
  save(draft: LiveSettingsDraft): Promise<RulesResult> {
    return this.run(draft, true);
  }

  private async run(draft: LiveSettingsDraft, save: boolean): Promise<RulesResult> {
    try {
      const { toml: before, model } = await this.edits.current();
      writeLive(model, draft);
      const toml = this.edits.toToml(model);
      const validation = await this.backend.validateProfile(toml);
      if (!validation.ok) {
        const line = validation.line ?? null;
        const excerpt = line ? (toml.split('\n')[line - 1]?.trim().slice(0, 160) ?? null) : null;
        return { ok: false, failure: { message: validation.error ?? $localize`:@@live.rulesProfile.invalid:The profile is not valid.`, line, excerpt } };
      }
      if (save) {
        this.store.profileSaved(await this.backend.saveProfile(toml));
        this.undo.record($localize`:@@live.rulesProfile.undo:live rules edit`, before, toml);
      }
      return { ok: true };
    } catch (e) {
      return { ok: false, failure: { message: describeError(e), line: null, excerpt: null } };
    }
  }
}
