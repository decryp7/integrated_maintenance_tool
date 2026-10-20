/** Pixels each time bucket gets on a plot. */
export const PX_PER_BUCKET = 2;
/** Bucket counts are rounded to this step, so a few pixels of resizing never re-query. */
export const BUCKET_STEP = 50;
export const MIN_BUCKETS = 50;
export const MAX_BUCKETS = 2000;
/** A resize (splitter drag, window resize) changes the bucket count once it has settled this long. */
export const RESIZE_SETTLE_MS = 150;

/** The bucket count for a plot `width` px wide: about one bucket per `PX_PER_BUCKET` px, in coarse steps, bounded. */
export function bucketsForWidth(width: number): number {
  if (!Number.isFinite(width) || width <= 0) return MIN_BUCKETS;
  const steps = Math.round(width / PX_PER_BUCKET / BUCKET_STEP);
  return Math.min(MAX_BUCKETS, Math.max(MIN_BUCKETS, steps * BUCKET_STEP));
}
