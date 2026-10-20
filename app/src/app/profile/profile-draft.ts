import { computed, effect, inject, Injectable, signal, untracked } from '@angular/core';
import { TomlError } from 'smol-toml';
import { ProfileDoc, ProfileValidation } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { Logger } from '../core/logger';
import { Platform } from '../core/platform';
import { ProfileModel } from '../core/profile.model';
import { WorkspaceStore } from '../core/workspace.store';
import { parseProfile, stringifyProfile } from './toml';
import { OverwriteGuard } from '../ui/overwrite-guard';
import { ProfileUndo } from '../undo/profile-undo';

/** The profile's `name`, when the text parses. */
function profileName(toml: string): string | null {
  try {
    return parseProfile(toml).name ?? null;
  } catch {
    return null;
  }
}

/**
 * The profile being edited. The TOML text is the source of truth; the form edits a parsed
 * copy and writes it back (which normalises formatting and drops comments).
 */
@Injectable()
export class ProfileDraft {
  private readonly backend = inject(ImtBackend);
  private readonly store = inject(WorkspaceStore);
  private readonly platform = inject(Platform);
  private readonly overwrite = inject(OverwriteGuard);
  private readonly logger = inject(Logger);
  private readonly undo = inject(ProfileUndo);

  readonly doc = signal<ProfileDoc | null>(null);
  readonly text = signal('');
  readonly model = signal<ProfileModel>({});
  readonly parseError = signal<{ message: string; line: number | null } | null>(null);
  readonly validation = signal<ProfileValidation | null>(null);
  readonly saving = signal(false);
  readonly importedFrom = signal<string | null>(null);
  /**
   * The active profile changed underneath unsaved edits (undo, redo, a restored version): the edits are kept
   * but are based on an older version; saving replaces the newer one.
   */
  readonly stale = signal(false);
  readonly dirty = computed(() => {
    const doc = this.doc();
    return !!doc && this.text() !== doc.toml;
  });
  readonly problem = computed(() => {
    const p = this.parseError();
    if (p) return p;
    const v = this.validation();
    return v && !v.ok ? { message: v.error ?? $localize`:@@profile.draft.invalid:Invalid profile`, line: v.line } : null;
  });

  private validateTimer: ReturnType<typeof setTimeout> | undefined;
  private validateSeq = 0;
  private seenProfileVersion = this.store.profileVersion();

  constructor() {
    // Follow profile changes made elsewhere (undo/redo, marker rules, a restored version).
    effect(() => {
      const version = this.store.profileVersion();
      untracked(() => {
        if (version === this.seenProfileVersion) return;
        this.seenProfileVersion = version;
        void this.refresh();
      });
    });
  }

  /** Takes the active profile when it differs from the loaded one: replaces a clean draft, flags a dirty one as stale. */
  async refresh(): Promise<void> {
    if (!this.doc()) return;
    try {
      const doc = await this.backend.getProfile();
      if (doc.toml === this.doc()?.toml) return;
      if (this.dirty()) {
        this.stale.set(true);
        this.store.setNotice($localize`:@@profile.draft.changed:The active profile changed: your unsaved profile edits are based on the previous version.`);
        return;
      }
      this.doc.set(doc);
      this.importedFrom.set(null);
      this.setText(doc.toml);
    } catch {
      /* the editor keeps what it has; the next load shows the error */
    }
  }

  async load(): Promise<void> {
    try {
      const doc = await this.backend.getProfile();
      this.doc.set(doc);
      this.importedFrom.set(null);
      this.stale.set(false);
      this.setText(doc.toml);
    } catch (e) {
      this.store.setError(String(e));
    }
  }

  setText(text: string): void {
    this.text.set(text);
    try {
      this.model.set(parseProfile(text));
      this.parseError.set(null);
    } catch (e) {
      const err = e as TomlError;
      this.parseError.set({ message: (err.message ?? String(e)).split('\n')[0] ?? '', line: err.line ?? null });
    }
    this.scheduleValidation();
  }

  /** Applies a form edit to a copy of the model and regenerates the TOML. */
  update(mutate: (m: ProfileModel) => void): void {
    const next = structuredClone(this.model());
    mutate(next);
    this.model.set(next);
    this.text.set(stringifyProfile(next));
    this.parseError.set(null);
    this.scheduleValidation();
  }

  private scheduleValidation(): void {
    clearTimeout(this.validateTimer);
    const text = this.text();
    const mine = ++this.validateSeq;
    this.validateTimer = setTimeout(async () => {
      try {
        const v = await this.backend.validateProfile(text);
        if (mine === this.validateSeq) this.validation.set(v);
      } catch (e) {
        if (mine === this.validateSeq) this.validation.set({ ok: false, error: String(e), line: null });
      }
    }, 300);
  }

  revert(): void {
    // Edits based on an older version go back to the active profile, not to the version they started from.
    if (this.stale()) {
      void this.load();
      return;
    }
    const doc = this.doc();
    if (doc) {
      this.importedFrom.set(null);
      this.setText(doc.toml);
    }
  }

  async save(): Promise<void> {
    if (this.problem()) return;
    this.saving.set(true);
    try {
      const text = this.text();
      // Saved now; the files re-import in the background (the status bar shows it).
      await this.undo.save($localize`:Undo history entry|@@profile.draft.undoLabel:profile edit`, text);
      this.seenProfileVersion = this.store.profileVersion() + 1;
      this.logger.info('ui.profile', 'Profile saved', { Bytes: text.length });
      this.doc.update((d) => ({ name: profileName(text) ?? d?.name ?? '', path: d?.path ?? null, toml: text }));
      this.importedFrom.set(null);
      this.stale.set(false);
    } catch (e) {
      this.store.setError($localize`:@@profile.draft.saveFailed:Profile not saved: ${String(e)}:error:`);
    } finally {
      this.saving.set(false);
    }
  }

  async importFile(): Promise<void> {
    try {
      const path = await this.platform.pickProfileToOpen();
      if (!path) return;
      const doc = await this.backend.importProfile(path);
      this.importedFrom.set(path);
      this.setText(doc.toml);
    } catch (e) {
      this.store.setError($localize`:@@profile.draft.importFailed:Import failed: ${String(e)}:error:`);
    }
  }

  async exportFile(): Promise<void> {
    try {
      const name = (this.model().name ?? 'profile').replace(/[^\w.-]+/g, '-').replace(/^-+|-+$/g, '') || 'profile';
      const path = await this.platform.pickProfileToSave(`${name}.toml`);
      if (!path) return;
      const text = this.text();
      await this.overwrite.write(path, (overwrite) => this.backend.exportProfile(path, text, overwrite));
    } catch (e) {
      this.store.setError($localize`:@@profile.draft.exportFailed:Export failed: ${String(e)}:error:`);
    }
  }
}
