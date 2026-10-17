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

/** The first-save notice. */
export const SNAPSHOT_NOTICE = 'IMT saved a screenshot of this error locally to help diagnose it. Nothing is sent anywhere.';
