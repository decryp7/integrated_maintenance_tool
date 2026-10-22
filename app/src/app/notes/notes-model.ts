import { NoteAnchor, NoteItem, NoteSeverity } from '../core/api.types';

/**
 * Pure presentation helpers of notes: filtering the list, grouping notes by what the views mark (rows, sources,
 * timeline), and short labels. What a note is and which anchors resolve is the engine's.
 */

export type NoteKind = NoteAnchor['kind'];
export const NOTE_KINDS: readonly NoteKind[] = ['event', 'instant', 'range', 'source', 'marker', 'chartPoint', 'project', 'image'];

export interface NoteFilter {
  /** null: every kind. */
  kind: NoteKind | null;
  /** null: any tag. */
  tag: string | null;
  /** Words that must all appear in the text, tags or anchor label (case-insensitive). */
  text: string;
}

export const NO_FILTER: NoteFilter = { kind: null, tag: null, text: '' };

export function kindLabel(kind: NoteKind): string {
  switch (kind) {
    case 'event':
      return $localize`:Note anchor kind|@@notes.kind.event:Event`;
    case 'instant':
      return $localize`:Note anchor kind|@@notes.kind.instant:Moment`;
    case 'range':
      return $localize`:Note anchor kind|@@notes.kind.range:Time range`;
    case 'source':
      return $localize`:Note anchor kind|@@notes.kind.source:Source`;
    case 'marker':
      return $localize`:Note anchor kind|@@notes.kind.marker:Marker`;
    case 'chartPoint':
      return $localize`:Note anchor kind|@@notes.kind.chartPoint:Chart point`;
    case 'project':
      return $localize`:Note anchor kind|@@notes.kind.project:Project`;
    case 'image':
      return $localize`:Note anchor kind|@@notes.kind.image:Image`;
  }
}

export function severityLabel(s: NoteSeverity): string {
  switch (s) {
    case 'info':
      return $localize`:Note severity|@@notes.severity.info:Info`;
    case 'warning':
      return $localize`:Note severity|@@notes.severity.warning:Warning`;
    case 'problem':
      return $localize`:Note severity|@@notes.severity.problem:Problem`;
  }
}

/** The theme token a note's severity is drawn with. */
export function severityColor(s: NoteSeverity): string {
  return s === 'problem' ? 'var(--lvl-error)' : s === 'warning' ? 'var(--lvl-warn)' : 'var(--accent)';
}

const baseName = (p: string) => p.split(/[\\/]/).filter(Boolean).at(-1) ?? p;

/** What the note is about, in a few words (no times: views format those in the user's time zone). */
export function anchorText(a: NoteAnchor): string {
  switch (a.kind) {
    case 'event':
      return a.excerpt || $localize`:@@notes.anchor.event:Event ${a.eventId}:id:`;
    case 'source':
      return baseName(a.path);
    case 'marker':
      return a.name;
    case 'chartPoint':
      return a.series ? `${a.chart} · ${a.series}` : a.chart;
    case 'image':
      return a.image.name;
    case 'project':
      return $localize`:@@notes.anchor.project:The whole project`;
    case 'instant':
    case 'range':
      return kindLabel(a.kind);
  }
}

/** The time a note covers (start, end), if any. */
export function anchorSpan(a: NoteAnchor): [number, number] | null {
  switch (a.kind) {
    case 'event':
      return a.ts == null ? null : [a.ts, a.ts];
    case 'instant':
    case 'chartPoint':
      return [a.at, a.at];
    case 'range':
      return [a.from, a.to];
    case 'marker':
      return [a.from, a.to ?? a.from];
    default:
      return null;
  }
}

export function matches(n: NoteItem, f: NoteFilter): boolean {
  if (f.kind && n.anchor.kind !== f.kind) return false;
  if (f.tag && !n.tags.includes(f.tag)) return false;
  const words = f.text.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const hay = `${n.text} ${n.tags.join(' ')} ${anchorText(n.anchor)}`.toLowerCase();
  return words.every((w) => hay.includes(w));
}

/** Every tag in use, sorted. */
export function allTags(notes: readonly NoteItem[]): string[] {
  return [...new Set(notes.flatMap((n) => n.tags))].sort();
}

/** Notes by event id (rows with notes show a glyph). */
export function byEvent(notes: readonly NoteItem[]): ReadonlyMap<number, NoteItem[]> {
  const m = new Map<number, NoteItem[]>();
  for (const n of notes) if (n.anchor.kind === 'event') m.set(n.anchor.eventId, [...(m.get(n.anchor.eventId) ?? []), n]);
  return m;
}

/** How many notes each source path has (the Explorer's badge). */
export function bySource(notes: readonly NoteItem[]): ReadonlyMap<string, number> {
  const m = new Map<string, number>();
  for (const n of notes) if (n.anchor.kind === 'source') m.set(n.anchor.path, (m.get(n.anchor.path) ?? 0) + 1);
  return m;
}

/** A note drawn on the timeline: where, how and what it says. */
export interface TimeNote {
  id: string;
  from: number;
  to: number;
  severity: NoteSeverity;
  label: string;
}

/** Notes on times and ranges (moments, ranges, markers, chart points) that overlap `[from, to]`. Event notes are marked on their rows instead. */
export function timeNotes(notes: readonly NoteItem[], from: number, to: number): TimeNote[] {
  const out: TimeNote[] = [];
  for (const n of notes) {
    if (n.anchor.kind === 'event') continue;
    const span = anchorSpan(n.anchor);
    if (!span || span[1] < from || span[0] > to) continue;
    out.push({ id: n.id, from: span[0], to: span[1], severity: n.severity, label: firstLine(n.text) });
  }
  return out;
}

/** The first line of a note's text, without Markdown heading or list marks, at most `max` characters. */
export function firstLine(text: string, max = 80): string {
  const line = (text.split('\n').find((l) => l.trim()) ?? '')
    .replace(/^\s*(#+|[-*+]|\d+\.)\s+/, '')
    .replace(/\*\*|__|`/g, '')
    .trim();
  return line.length > max ? `${line.slice(0, max - 1)}…` : line;
}
