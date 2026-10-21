/**
 * Pure display helpers for images and their related logs (ADR 0025). The engine finds, ranks and explains the links;
 * these only put its typed reasons and notes into words and keep its order.
 */
import { formatOffset } from '../../skew/skew-format';
import { AiImage, ImageTime, ImageTimeSource, Narrative, RelatedLink, RelatedNote, RelatedOutcome, RelatedReason, ScreenKind } from '../ai.types';

/** An event linked to an image, with why: what a "linked to image" note will hold (ADR 0021). */
export interface ImageLink {
  imageId: number | null;
  imageName: string | null;
  eventId: number;
  reasons: RelatedReason[];
}

/** The links of an outcome as `ImageLink`s, in the engine's order. */
export function imageLinks(imageId: number | null, imageName: string | null, outcome: RelatedOutcome): ImageLink[] {
  return outcome.result.links.map((l) => ({ imageId, imageName, eventId: l.event.id, reasons: l.reasons }));
}

/** "time +1.4 s", "title matches lane OrderWorker", "text 'HttpRequestException'", "text close to '…'". */
export function reasonLabel(r: RelatedReason): string {
  switch (r.kind) {
    case 'time': {
      const offset = formatOffset(r.deltaMs);
      return $localize`:@@ai.vision.reason.time:time ${offset}:offset:`;
    }
    case 'title': {
      const t = r.target;
      if (t.kind === 'source') return $localize`:@@ai.vision.reason.titleSource:title matches source ${t.name}:name:`;
      if (t.kind === 'lane') return $localize`:@@ai.vision.reason.titleLane:title matches lane ${t.lane}:lane:`;
      return $localize`:@@ai.vision.reason.titleField:title matches ${t.field}:field: ${t.value}:value:`;
    }
    case 'text':
      return r.partial ? $localize`:@@ai.vision.reason.textPartial:text close to '${r.clue}:clue:'` : $localize`:@@ai.vision.reason.text:text '${r.clue}:clue:'`;
  }
}

/** A note of the search in words; `time` formats an epoch-ms time for the user's time zone. */
export function noteLabel(n: RelatedNote, time: (ms: number | null) => string): string {
  switch (n.kind) {
    case 'noTime':
      return $localize`:@@ai.vision.note.noTime:No readable time on the image: matched by text and title only.`;
    case 'outsideData': {
      const from = time(n.from);
      const to = time(n.to);
      return $localize`:@@ai.vision.note.outsideData:The image's time is outside the loaded data (${from}:from: – ${to}:to:).`;
    }
    case 'clockDisagreement': {
      const offset = formatOffset(n.deltaMs);
      return $localize`:@@ai.vision.note.clock:The clocks likely disagree: the best match in ${n.sourceName}:source: is ${offset}:offset: away from the image's time.`;
    }
    case 'offsetApplied': {
      const offset = formatOffset(n.offsetMs);
      return $localize`:@@ai.vision.note.offsetApplied:Applied the clock offset of ${n.sourceName}:source: (${offset}:offset:).`;
    }
    case 'nothingToSearch':
      return $localize`:@@ai.vision.note.nothing:The model found nothing on the image to search for.`;
  }
}

function timeSourceLabel(s: ImageTimeSource): string {
  switch (s) {
    case 'screen':
      return $localize`:@@ai.vision.time.screen:shown on the screen`;
    case 'screenTimeOfDay':
      return $localize`:@@ai.vision.time.screenTimeOfDay:time of day shown on the screen`;
    case 'exif':
      return $localize`:@@ai.vision.time.exif:the photo's capture time`;
    case 'fileName':
      return $localize`:@@ai.vision.time.fileName:from the file name`;
    case 'fileModified':
      return $localize`:@@ai.vision.time.fileModified:the file's modification time`;
  }
}

/** "Taken at 14 Oct 10:41:26 (shown on the screen)". */
export function imageTimeLabel(t: ImageTime, time: (ms: number | null) => string): string {
  const at = time(t.atMs);
  const source = timeSourceLabel(t.source);
  return $localize`:@@ai.vision.time.label:Taken at ${at}:time: (${source}:source:)`;
}

const KINDS: Record<ScreenKind, () => string> = {
  error_dialog: () => $localize`:@@ai.vision.kind.errorDialog:Error dialog`,
  frozen_window: () => $localize`:@@ai.vision.kind.frozenWindow:Frozen window`,
  console: () => $localize`:@@ai.vision.kind.console:Console`,
  dashboard: () => $localize`:@@ai.vision.kind.dashboard:Dashboard`,
  log_viewer: () => $localize`:@@ai.vision.kind.logViewer:Log viewer`,
  browser: () => $localize`:@@ai.vision.kind.browser:Browser`,
  other: () => $localize`:@@ai.vision.kind.other:Screenshot`,
};

export function screenKindLabel(kind: ScreenKind): string {
  return (KINDS[kind] ?? KINDS.other)();
}

/** "1366 × 768 · original 2732 × 1536 PNG". */
export function sizeLabel(img: AiImage): string {
  const fmt = img.sourceFormat.toUpperCase();
  if (img.width === img.originalWidth && img.height === img.originalHeight) return `${img.width} × ${img.height} · ${fmt}`;
  return $localize`:@@ai.vision.size:${img.width}:w: × ${img.height}:h: · original ${img.originalWidth}:ow: × ${img.originalHeight}:oh: ${fmt}:format:`;
}

/** "Captured 2026-10-14 10:41:26" (EXIF, as the device wrote it), or null. */
export function capturedLabel(img: AiImage): string | null {
  return img.capturedAt ? $localize`:@@ai.vision.captured:Captured ${img.capturedAt}:time:` : null;
}

/** The links grouped by source, groups and links in the engine's order (it already ranked them). */
export function groupBySource(links: readonly RelatedLink[]): { sourceName: string; links: RelatedLink[] }[] {
  const groups = new Map<string, RelatedLink[]>();
  for (const l of links) {
    const g = groups.get(l.event.sourceName);
    if (g) g.push(l);
    else groups.set(l.event.sourceName, [l]);
  }
  return [...groups].map(([sourceName, ls]) => ({ sourceName, links: ls }));
}

const isObject = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object';

/** A job's narrative result, or null when it is not one (job results arrive untyped). */
export function parseNarrative(v: unknown): Narrative | null {
  return isObject(v) && typeof v['text'] === 'string' && Array.isArray(v['citations']) ? (v as unknown as Narrative) : null;
}

/** A job's `relatedLogs` result, or null when it is not one. */
export function parseRelatedOutcome(v: unknown): RelatedOutcome | null {
  if (!isObject(v) || !isObject(v['result']) || !isObject(v['facts'])) return null;
  const r = v['result'];
  if (!Array.isArray(r['links']) || !Array.isArray(r['notes'])) return null;
  return parseNarrative(v['explanation']) ? (v as unknown as RelatedOutcome) : null;
}
