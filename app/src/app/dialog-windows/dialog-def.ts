import { DialogDef as KitDialogDef, DialogSpec, defineDialog as defineKitDialog } from './kit';

export * from './kit';

/** A group of providers a dialog window loads on top of the base set (see `dialog-bootstrap.ts`). */
export type DialogBackend = 'ai' | 'tools' | 'share' | 'updates' | 'diagnostics' | 'analysis' | 'profiling' | 'native' | 'dotnet' | 'workspace' | 'summary' | 'skew';

/** One dialog of Debrief: the kit's definition with Debrief's backend groups. */
export type DialogDef<Id extends string = string, C = unknown, R = unknown> = KitDialogDef<Id, DialogBackend, C, R>;

/** Declares one dialog of Debrief (ADR 0054). */
export function defineDialog<const Id extends string, C, R>(spec: DialogSpec<Id, DialogBackend, C, R>): DialogDef<Id, C, R> {
  return defineKitDialog(spec);
}

/**
 * The commands every dialog window of Debrief calls whatever it shows, besides the kit's own (taking its input,
 * answering, sizing): the UI state sync with the host, theme and display settings, logging and the activity log.
 * A dialog never lists these. `check-dialog-commands.mjs` derives them from the shell and the base providers.
 */
export const DIALOG_BASE_COMMANDS: readonly string[] = ['capture_window_image', 'diagnostics_settings', 'recent_list', 'save_error_snapshot'];

/**
 * What applying the host's synced UI state may call in a dialog window (`sharedUiState` in `popout/shared-state.ts`:
 * `live` restores the live state, `data` refreshes the summary, `query` reloads the timeline). The analysis does not
 * enter that function, so they are listed here.
 */
export const DIALOG_SYNC_COMMANDS: readonly string[] = ['live_state', 'workspace_summary', 'query_timeline'];

/**
 * Commands the providers of a backend group send by themselves when a window boots with it (the AI status store asks the
 * engine for its status as soon as it exists). Every dialog that names the group may send them; a dialog never lists them.
 * `dialog-commands-boot.spec.ts` finds them: it boots every dialog over the real Tauri backends and records what is sent.
 */
export const DIALOG_BACKEND_COMMANDS: Readonly<Partial<Record<DialogBackend, readonly string[]>>> = {
  ai: ['ai_status'],
};
