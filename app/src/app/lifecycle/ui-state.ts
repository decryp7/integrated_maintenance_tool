import { computed, inject, Injectable, InjectionToken, signal } from '@angular/core';
import { UiState } from '../core/tools.types';
import { flattenContributions } from '../extensions/tokens';
import { Logger } from '../core/logger';

/**
 * A feature's share of the serializable UI state saved in projects and autosaved sessions.
 * Contribute through the `UI_STATE_CONTRIBUTIONS` multi token (one batch per provider) or at
 * runtime with `UiStateRegistry.register`.
 */
export interface UiStateContribution {
  /** Key in the saved `ui` object; unique. */
  key: string;
  /** Reads the state. Called inside a computed, so reading signals here makes the state reactive. */
  capture(): unknown;
  /** Applies saved state. Receives whatever was stored under `key` (validate it). */
  restore(value: unknown): void;
  /** Called instead of `restore` when the saved state has no entry for `key` (new project, older file). */
  reset?(): void;
  /** False for transient state (selection, layout) that is saved but must not mark a project unsaved. */
  tracksChanges?: boolean;
}

export const UI_STATE_CONTRIBUTIONS = new InjectionToken<readonly UiStateContribution[]>('UI_STATE_CONTRIBUTIONS');

export const UI_STATE_VERSION = 1;

/** Collects the contributions and turns them into one `ui` value (and back). Failures stay contained per key. */
@Injectable({ providedIn: 'root' })
export class UiStateRegistry {
  private readonly logger = inject(Logger);
  private readonly injected = flattenContributions<UiStateContribution>(inject(UI_STATE_CONTRIBUTIONS, { optional: true }));
  private readonly added = signal<readonly UiStateContribution[]>([]);

  readonly contributions = computed(() => {
    const seen = new Set<string>();
    return [...this.injected, ...this.added()].filter((c) => !seen.has(c.key) && !!seen.add(c.key));
  });

  /** Registers a contribution at runtime. Returns a function that removes it again. */
  register(contribution: UiStateContribution): () => void {
    this.added.update((list) => [...list, contribution]);
    return () => this.added.update((list) => list.filter((c) => c !== contribution));
  }

  /** The state to save. A contribution that throws is left out (and logged), never fails the save. */
  serialize(): UiState {
    return this.capture(this.contributions());
  }

  /** Applies saved state (missing keys reset their contribution). Malformed input counts as empty; a failing contribution does not stop the others. */
  restore(ui: unknown): void {
    const saved = ui !== null && typeof ui === 'object' && !Array.isArray(ui) ? (ui as Record<string, unknown>) : {};
    for (const c of this.contributions()) {
      try {
        if (c.key in saved) c.restore(saved[c.key]);
        else c.reset?.();
      } catch (e) {
        this.logger.warn('ui.state', 'Saved UI state could not be restored', { Key: c.key, Error: e instanceof Error ? e.message : String(e) });
      }
    }
  }

  /** Everything, as JSON: changes when anything changes (autosave). */
  readonly snapshot = computed(() => JSON.stringify(this.serialize()));

  /** Only the state whose changes mean "unsaved changes" (project dirty flag). */
  readonly trackedSnapshot = computed(() => JSON.stringify(this.capture(this.contributions().filter((c) => c.tracksChanges !== false))));

  private capture(list: readonly UiStateContribution[]): UiState {
    const ui: UiState = { version: UI_STATE_VERSION };
    for (const c of list) {
      try {
        ui[c.key] = c.capture();
      } catch (e) {
        this.logger.warn('ui.state', 'UI state could not be captured', { Key: c.key, Error: e instanceof Error ? e.message : String(e) });
      }
    }
    return ui;
  }
}
