/**
 * Wire types of the updater (mirror `app/src-tauri/src/updater.rs`, `updater/state.rs`, `updater/error.rs`).
 *
 *   updates_status                      -> UpdateStatus
 *   updates_check                       -> UpdateStatus   (rejects with UpdateError)
 *   updates_download                    -> UpdateStatus   (progress arrives as events)
 *   updates_cancel                      -> UpdateStatus
 *   updates_install_on_quit { enabled } -> UpdateStatus
 *   updates_restart                     -> void           (IMT quits, installs and starts again)
 *   updates_open_releases               -> void
 *   event imt://updates                 -> UpdateStatus   (to every window)
 */

export const UPDATES_EVENT = 'imt://updates';

export type UpdateErrorKind =
  | 'notAvailable'
  | 'offline'
  | 'unreachable'
  | 'noRelease'
  | 'noPlatform'
  | 'invalidManifest'
  | 'signatureInvalid'
  | 'tooLarge'
  | 'diskFull'
  | 'installFailed'
  | 'busy';

/** A failure with the text to show (written by the engine) and the next step. */
export interface UpdateError {
  /** `unknown`: the request itself failed (not an updater error). */
  kind: UpdateErrorKind | 'unknown';
  message: string;
  nextStep: string;
  detail: string | null;
}

export interface UpdateInfo {
  version: string;
  currentVersion: string;
  /** Plain text: shown as text, never as markup. */
  notes: string;
  /** RFC 3339. */
  pubDate: string | null;
  /** Bytes, when the release says. */
  size: number | null;
}

export type UpdatePhase =
  | { kind: 'unavailable' }
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'upToDate' }
  | { kind: 'available'; info: UpdateInfo }
  | { kind: 'downloading'; info: UpdateInfo; downloaded: number; total: number | null }
  | { kind: 'ready'; info: UpdateInfo; installOnQuit: boolean }
  | { kind: 'installing'; info: UpdateInfo }
  | { kind: 'failed'; error: UpdateError; info: UpdateInfo | null };

export interface UpdateStatus {
  currentVersion: string;
  phase: UpdatePhase;
  /** RFC 3339. */
  lastChecked: string | null;
  autoCheck: boolean;
  channel: 'stable';
  /** The host a check contacts (null in a build without updates). */
  endpointHost: string | null;
  releasesPage: string;
}

/** Turns whatever a rejected request carries into an `UpdateError`. */
export function toUpdateError(e: unknown): UpdateError {
  if (e && typeof e === 'object' && typeof (e as UpdateError).kind === 'string' && typeof (e as UpdateError).message === 'string') {
    const u = e as UpdateError;
    return { kind: u.kind, message: u.message, nextStep: u.nextStep ?? '', detail: u.detail ?? null };
  }
  const message = typeof e === 'string' ? e : e instanceof Error ? e.message : $localize`:@@updates.requestFailed:The request failed.`;
  return { kind: 'unknown', message, nextStep: $localize`:@@updates.tryAgain:Try again.`, detail: null };

}
