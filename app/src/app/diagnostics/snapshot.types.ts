/**
 * UI-only types of the error snapshot feature. The wire contract (SnapshotContext, SnapshotInfo,
 * SnapshotSaveResult, EngineError, ...) lives in `core/api.types.ts`.
 */

/** A rendered window image, ready to send. */
export interface CapturedImage {
  /** Base64, without the `data:` prefix. */
  base64: string;
  mime: 'image/png' | 'image/jpeg';
  bytes: number;
}

/** A transparent 1 × 1 PNG: the picture of a snapshot taken where the window could not be captured. */
export const NO_SCREENSHOT_IMAGE: CapturedImage = {
  base64: 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNgYGBgAAAABQABeqhXUAAAAABJRU5ErkJggg==',
  mime: 'image/png',
  bytes: 70,
};

/** What a snapshot without a picture says about it; `reason` comes from the shell. */
export const noScreenshotNote = (reason: string) => $localize`:@@diagnostics.snapshot.noScreenshot:No screenshot: ${reason}:reason:.`;

/** Set on `<html>` while the shell captures the window; elements marked `data-snapshot-ignore` are hidden meanwhile. */
export const CAPTURING_ATTRIBUTE = 'data-snapshot-capturing';

/** The first-save notice. */
export const SNAPSHOT_NOTICE = $localize`:@@diagnostics.snapshot.notice:Debrief saved a screenshot of this error locally to help diagnose it. Nothing is sent anywhere.`;

/** Help ▸ Report a problem… */
export const REPORT_PROBLEM_COMMAND = 'help.reportProblem';
/** Help ▸ Show activity log */
export const ACTIVITY_LOG_COMMAND = 'help.activityLog';
