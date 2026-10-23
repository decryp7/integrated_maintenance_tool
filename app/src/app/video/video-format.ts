import { CmdError } from '../core/cmd-error';
import { DateFormat, formatDateTime, TimeZoneMode } from '../core/format';
import { plural } from '../i18n/locale';
import { AlignEvidence, Alignment, FrameRecord, SelectNote, SelectOptions, UserAlignment, VideoRecord } from '../core/generated/video';

/** Extensions offered by the open dialog and used to pick dropped files worth sniffing. */
export const VIDEO_EXTENSIONS = ['mp4', 'm4v', 'mov', 'mkv', 'webm', 'avi'] as const;

/** Whether the file name has a video extension (the engine still sniffs the content). */
export function hasVideoExtension(path: string): boolean {
  const dot = path.lastIndexOf('.');
  return dot >= 0 && (VIDEO_EXTENSIONS as readonly string[]).includes(path.slice(dot + 1).toLowerCase());
}

const pad = (n: number, width = 2) => String(n).padStart(width, '0');

/** A time in a video: `01:23` (`1:02:03` past an hour), with milliseconds when `precise`. */
export function formatVideoTime(ms: number, precise = false): string {
  const total = Math.max(0, Math.floor(ms));
  const h = Math.floor(total / 3_600_000);
  const m = Math.floor((total % 3_600_000) / 60_000);
  const s = Math.floor((total % 60_000) / 1000);
  const base = h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
  return precise ? `${base}.${pad(total % 1000, 3)}` : base;
}

/** The name of a frame in lists: "Frame 3 at 00:12". */
export function frameName(index: number, frame: Pick<FrameRecord, 'videoMs'>): string {
  const n = index + 1;
  const at = formatVideoTime(frame.videoMs);
  return $localize`:@@video.frame.name:Frame ${n}:n: at ${at}:time:`;
}

/** The moment of a frame on the logs' clock. */
export function frameAlignedMs(record: Pick<VideoRecord, 'alignment'>, frame: Pick<FrameRecord, 'videoMs'>): number {
  return record.alignment.startMs + frame.videoMs;
}

export function alignedByLabel(by: Alignment['by']): string {
  switch (by) {
    case 'user':
      return $localize`:@@video.align.by.user:set by you`;
    case 'clock':
      return $localize`:@@video.align.by.clock:the clock on the screen`;
    case 'fileName':
      return $localize`:@@video.align.by.fileName:the date in the file name`;
    case 'container':
      return $localize`:@@video.align.by.container:the video's own creation time`;
    case 'fileTime':
      return $localize`:@@video.align.by.fileTime:the file's modification time`;
    case 'none':
      return $localize`:@@video.align.by.none:nothing (no time was found)`;
  }
}

export function confidenceLabel(c: Alignment['confidence']): string {
  switch (c) {
    case 'high':
      return $localize`:@@video.align.confidence.high:high`;
    case 'medium':
      return $localize`:@@video.align.confidence.medium:medium`;
    case 'low':
      return $localize`:@@video.align.confidence.low:low`;
  }
}

/** The evidence of an alignment as a sentence ("the clock in the frame at 00:12 read 10:42:07"). */
export function evidenceText(e: AlignEvidence, tz: TimeZoneMode, style: DateFormat = 'readable'): string {
  switch (e.kind) {
    case 'userStart':
      return $localize`:@@video.evidence.userStart:You set when the video started.`;
    case 'userOffset': {
      const seconds = formatOffsetSeconds(e.offsetMs);
      const base = evidenceText(e.base, tz, style);
      return $localize`:@@video.evidence.userOffset:You moved the start by ${seconds}:seconds: s. Before that: ${base}:base:`;
    }
    case 'clock': {
      const at = formatVideoTime(e.frameMs);
      const text = e.text;
      return e.hasDate
        ? $localize`:@@video.evidence.clockDate:The clock in the frame at ${at}:time: read ${text}:text:, with its date.`
        : $localize`:@@video.evidence.clockTime:The clock in the frame at ${at}:time: read ${text}:text:; the date comes from the file.`;
    }
    case 'fileName': {
      const text = e.text;
      return $localize`:@@video.evidence.fileName:The file name contains the date and time ${text}:text:.`;
    }
    case 'container': {
      const created = formatDateTime(e.createdMs, tz, false, style);
      return $localize`:@@video.evidence.container:The video says it was created at ${created}:time:; some recorders write the end instead of the start.`;
    }
    case 'fileTime': {
      const modified = formatDateTime(e.modifiedMs, tz, false, style);
      return $localize`:@@video.evidence.fileTime:The file was last changed at ${modified}:time:; the start is that time minus the video's length.`;
    }
    case 'nothing':
      return $localize`:@@video.evidence.nothing:No time was found in the video or the file. Set the start yourself or read the clock from a frame.`;
  }
}

/** What the user told Debrief about the start, in words. */
export function userAlignmentText(u: UserAlignment, tz: TimeZoneMode, style: DateFormat = 'readable'): string {
  switch (u.kind) {
    case 'auto':
      return $localize`:@@video.align.user.auto:Automatic`;
    case 'start': {
      const at = formatDateTime(u.startMs, tz, true, style);
      return $localize`:@@video.align.user.start:Start set to ${at}:time:`;
    }
    case 'offset': {
      const seconds = formatOffsetSeconds(u.offsetMs);
      return $localize`:@@video.align.user.offset:Moved by ${seconds}:seconds: s`;
    }
  }
}

/** Signed seconds with up to three decimals: `+1.5`, `-30`. */
export function formatOffsetSeconds(ms: number): string {
  const s = Math.round(ms) / 1000;
  return `${s > 0 ? '+' : ''}${s}`;
}

/** The way frames were chosen, in words. */
export function selectionText(o: SelectOptions): string {
  const cap = o.cap;
  if (o.mode.mode === 'changes') {
    const threshold = o.mode.threshold;
    return $localize`:@@video.select.changes:Frames where the picture changed (threshold ${threshold}:threshold:), at most ${cap}:cap:.`;
  }
  const every = formatVideoTime(o.mode.everyMs);
  return $localize`:@@video.select.interval:One frame every ${every}:every:, at most ${cap}:cap:.`;
}

/** A note of the selection, explained. */
export function selectionNoteText(note: SelectNote): string {
  switch (note.kind) {
    case 'thresholdRaised': {
      const from = note.from;
      const to = note.to;
      return $localize`:@@video.select.note.raised:There were more changes than frames allowed, so the threshold was raised from ${from}:from: to ${to}:to:.`;
    }
    case 'spreadEvenly': {
      const n = note.leftOut;
      return plural(n, {
        one: $localize`:@@video.select.note.spread.one:Still too many: ${n}:count: change was left out and the frames kept are spread evenly over the video.`,
        other: $localize`:@@video.select.note.spread.other:Still too many: ${n}:count: changes were left out and the frames kept are spread evenly over the video.`,
      });
    }
    case 'noChanges':
      return $localize`:@@video.select.note.none:Nothing changed on the screen after the first frame; only the first and the last frames were kept.`;
  }
}

/** What removing a video deletes, for the confirmation. */
export interface Removal {
  frames: number;
  notes: number;
  descriptions: number;
  included: boolean;
}

export function removalOf(record: VideoRecord): Removal {
  return {
    frames: record.frames.length,
    notes: record.frames.filter((f) => !!f.note).length,
    descriptions: record.frames.filter((f) => !!f.description).length,
    included: record.included,
  };
}

/** The sentence of the removal confirmation. */
export function removalText(name: string, r: Removal): string {
  const { frames, notes, descriptions } = r;
  const counts = $localize`:@@video.remove.counts:${frames}:frames: frames, ${notes}:notes: notes and ${descriptions}:descriptions: descriptions`;
  const copy = r.included ? ' ' + $localize`:@@video.remove.copy:The copy of the video kept in the project goes too.` : '';
  return $localize`:@@video.remove.text:Remove ${name}:name: from the project? This deletes its ${counts}:counts:.${copy}:copy: The original video file is not touched.`;
}

// ---- Date-time inputs ----

/** `2026-10-22T10:42:07` for a `datetime-local` input, in the displayed time zone. */
export function toInputValue(ms: number, tz: TimeZoneMode): string {
  const d = new Date(ms);
  const p =
    tz === 'utc'
      ? [d.getUTCFullYear(), d.getUTCMonth() + 1, d.getUTCDate(), d.getUTCHours(), d.getUTCMinutes(), d.getUTCSeconds()]
      : [d.getFullYear(), d.getMonth() + 1, d.getDate(), d.getHours(), d.getMinutes(), d.getSeconds()];
  return `${p[0]}-${pad(p[1]!)}-${pad(p[2]!)}T${pad(p[3]!)}:${pad(p[4]!)}:${pad(p[5]!)}`;
}

/** Epoch ms of a `datetime-local` value in the displayed time zone; null when it is empty or not a date. */
export function fromInputValue(value: string, tz: TimeZoneMode): number | null {
  const m = /^(\d{4,})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::(\d{2}))?$/.exec(value.trim());
  if (!m) return null;
  const [y, mo, d, h, mi, s] = [Number(m[1]), Number(m[2]), Number(m[3]), Number(m[4]), Number(m[5]), Number(m[6] ?? 0)];
  const ms = tz === 'utc' ? Date.UTC(y, mo - 1, d, h, mi, s) : new Date(y, mo - 1, d, h, mi, s).getTime();
  return Number.isFinite(ms) ? ms : null;
}

/** Seconds typed by the user (decimal point or comma) as ms; null for anything that is not a number. */
export function parseOffsetSeconds(value: string): number | null {
  const t = value.trim().replace(',', '.');
  if (!/^[+-]?\d+(\.\d+)?$/.test(t)) return null;
  return Math.round(Number(t) * 1000);
}

// ---- Failures ----

/** Error codes that mean the AI cannot do this now: the fix is in Settings ▸ AI. */
const AI_SETUP_CODES = new Set(['noModel', 'noRuntime', 'noVision', 'engine', 'memory', 'modelFile']);

/** A failure for the panel: its message, the engine's next step and whether Settings ▸ AI is the place to fix it. */
export interface VideoFailure {
  message: string;
  hint: string | null;
  aiSetup: boolean;
  retryable: boolean;
}

export function toFailure(e: unknown): VideoFailure {
  if (e instanceof CmdError) return { message: e.message, hint: e.hint ?? null, aiSetup: AI_SETUP_CODES.has(e.code), retryable: e.retryable };
  return { message: e instanceof Error ? e.message : String(e), hint: null, aiSetup: false, retryable: true };
}
