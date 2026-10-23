import { DiagnosticRow, FixEvaluation, ProblemKind, QuickFix, Severity } from '../core/api.types';
import { formatCount } from '../core/format';
import { plural } from '../i18n/locale';

/** The Problems pane's view id. */
export const PROBLEMS_VIEW = 'problems';

/** One diagnostic row with a stable key (the pane's selection and tree ids). */
export interface ProblemItem {
  key: string;
  row: DiagnosticRow;
}

/** The problems of one file: a source, or a file no parser read. */
export interface ProblemGroup {
  key: string;
  name: string;
  path: string;
  /** The source id; null for a file that was not imported. */
  source: number | null;
  items: ProblemItem[];
  /** The most urgent severity of its problems (the group's icon). */
  severity: Severity;
}

/** Problem entries by severity, as the tab title and the status-bar badge count them. */
export interface ProblemCounts {
  critical: number;
  warning: number;
  info: number;
  total: number;
}

const RANK: Record<Severity, number> = { critical: 0, warning: 1, info: 2 };

/** A row's key: its file and kind (one row per source and kind; one per file not imported). */
export function problemKey(row: DiagnosticRow): string {
  return `${row.source ?? `file:${row.path}`}/${row.kind}`;
}

/** The rows grouped by file in the engine's order, each group's rows most urgent first. */
export function groupProblems(rows: readonly DiagnosticRow[]): ProblemGroup[] {
  const groups = new Map<string, ProblemGroup>();
  for (const row of rows) {
    const key = row.source !== null ? `source:${row.source}` : `file:${row.path}`;
    let group = groups.get(key);
    if (!group) {
      group = { key, name: row.sourceName, path: row.path, source: row.source, items: [], severity: row.severity };
      groups.set(key, group);
    }
    group.items.push({ key: problemKey(row), row });
    if (RANK[row.severity] < RANK[group.severity]) group.severity = row.severity;
  }
  for (const g of groups.values()) g.items.sort((a, b) => RANK[a.row.severity] - RANK[b.row.severity]);
  return [...groups.values()];
}

export function countProblems(rows: readonly DiagnosticRow[]): ProblemCounts {
  const counts: ProblemCounts = { critical: 0, warning: 0, info: 0, total: rows.length };
  for (const r of rows) counts[r.severity]++;
  return counts;
}

/** The tab title: "Problems" or "Problems (3)". */
export function problemsTitle(counts: ProblemCounts): string {
  const title = $localize`:View title@@problems.view.title:Problems`;
  return counts.total > 0 ? `${title} (${formatCount(counts.total)})` : title;
}

/** The kind of a problem in words. */
export function kindLabel(kind: ProblemKind): string {
  switch (kind) {
    case 'lineTooLong':
      return $localize`:@@problems.kind.lineTooLong:Lines over the size limit`;
    case 'eventTooLarge':
      return $localize`:@@problems.kind.eventTooLarge:Events over the size limit`;
    case 'lineNotParsed':
      return $localize`:@@problems.kind.lineNotParsed:Lines that are not events`;
    case 'timestampUnparsed':
      return $localize`:@@problems.kind.timestampUnparsed:Events without a readable timestamp`;
    case 'fieldInvalid':
      return $localize`:@@problems.kind.fieldInvalid:Field values of the wrong type`;
    case 'encoding':
      return $localize`:@@problems.kind.encoding:Bytes invalid in the file's encoding`;
    case 'unknownFormat':
      return $localize`:@@problems.kind.unknownFormat:Not imported: no parser reads it`;
    case 'truncatedArchive':
      return $localize`:@@problems.kind.truncatedArchive:Damaged or incomplete archive`;
    case 'eventSplit':
      return $localize`:@@problems.kind.eventSplit:Events split while reading in parallel`;
    case 'unmatchedLine':
      return $localize`:@@problems.kind.unmatchedLine:Lines that look like events but match no format`;
    case 'routeLimit':
      return $localize`:@@problems.kind.routeLimit:Events over the route limit`;
  }
}

/** What happened to the data, and what the user can do (shown under the kind). */
export function kindHint(kind: ProblemKind): string {
  switch (kind) {
    case 'timestampUnparsed':
      return $localize`:@@problems.hint.timestampUnparsed:They are in the grid but not on the timeline. A timestamp format in the file's parser fixes them.`;
    case 'lineNotParsed':
      return $localize`:@@problems.hint.lineNotParsed:They were skipped. A regex or template in the file's parser may read them.`;
    case 'unmatchedLine':
      return $localize`:@@problems.hint.unmatchedLine:They were kept with the event above. A regex or template in the file's parser may read them.`;
    case 'unknownFormat':
      return $localize`:@@problems.hint.unknownFormat:Teach Debrief the format with the format wizard, or ask the AI for a parser.`;
    case 'lineTooLong':
    case 'eventTooLarge':
      return $localize`:@@problems.hint.tooLarge:The rest was cut off at the limit.`;
    case 'fieldInvalid':
      return $localize`:@@problems.hint.fieldInvalid:The values were kept as text.`;
    case 'encoding':
      return $localize`:@@problems.hint.encoding:The invalid bytes were replaced.`;
    case 'truncatedArchive':
      return $localize`:@@problems.hint.truncatedArchive:The readable part was imported.`;
    case 'eventSplit':
      return $localize`:@@problems.hint.eventSplit:Each part is its own event.`;
    case 'routeLimit':
      return $localize`:@@problems.hint.routeLimit:They stay in the file's own source.`;
  }
}

/** Whether a parser change can fix this kind (offers "Ask the AI to fix"). */
export function parserCanFix(kind: ProblemKind): boolean {
  return kind === 'timestampUnparsed' || kind === 'lineNotParsed' || kind === 'unmatchedLine' || kind === 'unknownFormat';
}

/** "1,204 events" / "1 line": the row's count with its unit. */
export function countText(row: DiagnosticRow): string {
  const n = formatCount(row.count);
  if (row.kind === 'unknownFormat') return '';
  const events = row.kind === 'timestampUnparsed' || row.kind === 'eventTooLarge' || row.kind === 'eventSplit' || row.kind === 'routeLimit' || row.kind === 'fieldInvalid';
  return events
    ? plural(row.count, { one: $localize`:@@problems.count.event.one:${n}:count: event`, other: $localize`:@@problems.count.event.other:${n}:count: events` })
    : plural(row.count, { one: $localize`:@@problems.count.line.one:${n}:count: line`, other: $localize`:@@problems.count.line.other:${n}:count: lines` });
}

/** The engine's fix of a row as a quick fix for its source (null when it has none). */
export function quickFixOf(row: DiagnosticRow): QuickFix | null {
  if (!row.fix) return null;
  return { parserOverride: row.fix.parserOverride, sources: row.source !== null ? [row.source] : [] };
}

/** "Fixes 1,204 of 1,204 events (30 not checked)". */
export function evaluationText(e: FixEvaluation): string {
  const fixed = formatCount(e.fixed);
  const checked = formatCount(e.checked);
  const main =
    e.checked === 0
      ? $localize`:@@problems.eval.none:No stored events to check`
      : $localize`:@@problems.eval.fixes:Fixes ${fixed}:fixed: of ${checked}:checked: checked events`;
  if (e.unchecked === 0) return main;
  const unchecked = formatCount(e.unchecked);
  return `${main} ${$localize`:@@problems.eval.unchecked:(${unchecked}:unchecked: more not checked)`}`;
}

/** How good the preview is: everything checked fixed, some, or none. */
export function evaluationTone(e: FixEvaluation): 'good' | 'partial' | 'none' {
  if (e.checked === 0 || e.fixed === 0) return 'none';
  return e.fixed === e.checked ? 'good' : 'partial';
}
