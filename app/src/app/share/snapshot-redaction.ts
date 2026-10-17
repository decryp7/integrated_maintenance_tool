import { inject } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { SnapshotTextFilter } from '../diagnostics/error-snapshot.service';
import { ShareBackend } from './share-backend';

/** Stored instead of a text that could not be redacted (never the original). */
export const UNREDACTED_PLACEHOLDER = '[not stored: redaction failed]';

/**
 * Error snapshot texts follow the window's redacted view: while it is on, message and stack are
 * redacted by the engine with the profile's rules before they are stored next to the screenshot.
 */
export function snapshotRedaction(): SnapshotTextFilter {
  const store = inject(WorkspaceStore);
  const backend = inject(ShareBackend);
  return async (text: string) => {
    if (!store.summary()?.redactedView) return text;
    try {
      return (await backend.previewRedaction({ sample: text })).after;
    } catch {
      return UNREDACTED_PLACEHOLDER;
    }
  };
}
