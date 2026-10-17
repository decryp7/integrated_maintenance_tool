import { computed, inject, Injectable, signal } from '@angular/core';
import { parse, stringify, TomlError } from 'smol-toml';
import { ProfileDoc, ProfileValidation } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { Logger } from '../core/logger';
import { Platform } from '../core/platform';
import { ProfileModel } from '../core/profile.model';
import { WorkspaceStore } from '../core/workspace.store';

/** Removes undefined values (TOML has no null) so stringify never fails. */
function prune(value: unknown): unknown {
  if (Array.isArray(value)) return value.filter((v) => v !== undefined).map(prune);
  if (value && typeof value === 'object') {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value)) if (v !== undefined) out[k] = prune(v);
    return out;
  }
  return value;
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
  private readonly logger = inject(Logger);

  readonly doc = signal<ProfileDoc | null>(null);
  readonly text = signal('');
  readonly model = signal<ProfileModel>({});
  readonly parseError = signal<{ message: string; line: number | null } | null>(null);
  readonly validation = signal<ProfileValidation | null>(null);
  readonly saving = signal(false);
  readonly importedFrom = signal<string | null>(null);
  readonly dirty = computed(() => {
    const doc = this.doc();
    return !!doc && this.text() !== doc.toml;
  });
  readonly problem = computed(() => {
    const p = this.parseError();
    if (p) return p;
    const v = this.validation();
    return v && !v.ok ? { message: v.error ?? 'Invalid profile', line: v.line } : null;
  });

  private validateTimer: ReturnType<typeof setTimeout> | undefined;
  private validateSeq = 0;

  async load(): Promise<void> {
    try {
      const doc = await this.backend.getProfile();
      this.doc.set(doc);
      this.importedFrom.set(null);
      this.setText(doc.toml);
    } catch (e) {
      this.store.setError(String(e));
    }
  }

  setText(text: string): void {
    this.text.set(text);
    try {
      this.model.set(parse(text) as ProfileModel);
      this.parseError.set(null);
    } catch (e) {
      const err = e as TomlError;
      this.parseError.set({ message: (err.message ?? String(e)).split('\n')[0], line: err.line ?? null });
    }
    this.scheduleValidation();
  }

  /** Applies a form edit to a copy of the model and regenerates the TOML. */
  update(mutate: (m: ProfileModel) => void): void {
    const next = structuredClone(this.model());
    mutate(next);
    this.model.set(next);
    this.text.set(stringify(prune(next) as Record<string, unknown>));
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
      const summary = await this.backend.saveProfile(text);
      this.store.applySummary(summary);
      this.logger.info('ui.profile', 'Profile saved', { Markers: summary.markerKinds.length, Charts: summary.charts.length, Events: summary.total });
      this.doc.update((d) => ({ name: summary.profileName, path: d?.path ?? null, toml: text }));
      this.importedFrom.set(null);
    } catch (e) {
      this.store.setError(`Profile not saved: ${String(e)}`);
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
      this.store.setError(`Import failed: ${String(e)}`);
    }
  }

  async exportFile(): Promise<void> {
    try {
      const name = (this.model().name ?? 'profile').replace(/[^\w.-]+/g, '-').replace(/^-+|-+$/g, '') || 'profile';
      const path = await this.platform.pickProfileToSave(`${name}.toml`);
      if (!path) return;
      await this.backend.exportProfile(path, this.text());
    } catch (e) {
      this.store.setError(`Export failed: ${String(e)}`);
    }
  }
}
