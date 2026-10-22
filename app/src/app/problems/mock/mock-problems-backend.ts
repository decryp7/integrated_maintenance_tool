import { DiagnosticRow, FixEvaluation, IndexStatus, ProblemKind, QuickFix, Severity, SourceInfo } from '../../core/api.types';
import type { ImtBackend } from '../../core/backend';
import { CmdError } from '../../core/cmd-error';
import { MockBackend } from '../../core/mock/mock-backend';
import { parseFragment } from '../../core/mock/mock-parsers';
import type { ProblemsBackend } from '../problems-backend';

/** The engine's severity per kind (`ProblemKind::severity`), mimicked. */
const SEVERITY: Record<ProblemKind, Severity> = {
  lineTooLong: 'warning',
  eventTooLarge: 'warning',
  timestampUnparsed: 'warning',
  unknownFormat: 'warning',
  truncatedArchive: 'warning',
  unmatchedLine: 'warning',
  lineNotParsed: 'info',
  fieldInvalid: 'info',
  encoding: 'info',
  eventSplit: 'info',
  routeLimit: 'info',
};

/**
 * Browser stand-in for the import diagnostics commands, derived from the mock workspace's summary: each source's
 * problems, its timestamp diagnosis (with the engine's proposed format as the fix) and skipped lines, and the files no
 * parser read. A timestamp problem is gone once the source's parser has the proposed format (the fix was applied and
 * the file read again). Plausible, never the reference: the engine is.
 */
export class MockProblemsBackend implements ProblemsBackend {
  constructor(private readonly imt: ImtBackend) {}

  /** The TOML of the parser that read the source (empty when unknown). */
  private parserToml(s: SourceInfo): string {
    if (!(this.imt instanceof MockBackend) || !s.parser) return '';
    try {
      return this.imt.parsers.get(s.parser).toml;
    } catch {
      return '';
    }
  }

  private row(s: SourceInfo, kind: ProblemKind, count: number, sampleLines: number[]): DiagnosticRow {
    return { source: s.id, sourceName: s.name, path: s.path, kind, severity: SEVERITY[kind], count, sampleLines: sampleLines.slice(0, 20) };
  }

  async problems(): Promise<DiagnosticRow[]> {
    const summary = await this.imt.workspaceSummary();
    const out: DiagnosticRow[] = [];
    for (const s of [...summary.sources].sort((a, b) => a.name.localeCompare(b.name))) {
      const kinds = new Set<ProblemKind>();
      for (const p of s.problems ?? []) {
        kinds.add(p.kind);
        out.push(this.row(s, p.kind, p.count, p.sampleLines));
      }
      const issue = s.timestampIssue;
      const fixed = !!issue?.suggestion && this.parserToml(s).includes(issue.suggestion.format);
      if (issue && !fixed && !kinds.has('timestampUnparsed')) {
        const row = this.row(s, 'timestampUnparsed', issue.unparsedLines, issue.exampleLines);
        if (issue.suggestion) {
          row.fix = { title: `Read the timestamps as ${issue.suggestion.format}`, parserOverride: issue.suggestion.parserSnippet, timestamp: issue.suggestion };
        }
        out.push(row);
      }
      if (s.skipped > 0 && !kinds.has('lineNotParsed')) out.push(this.row(s, 'lineNotParsed', s.skipped, []));
    }
    for (const u of summary.unrecognized) {
      out.push({ source: null, sourceName: u.name, path: u.path, kind: 'unknownFormat', severity: SEVERITY.unknownFormat, count: 1, sampleLines: [], detail: u.reason });
    }
    return out;
  }

  /** Like the engine: an invalid fragment is refused; a fix "repairs" the untimed events when it adds the proposed format. */
  async evaluateFix(fix: QuickFix): Promise<FixEvaluation> {
    const over = parseFragment(fix.parserOverride);
    const format = over['format'];
    if (format !== undefined && (typeof format !== 'object' || format === null || Array.isArray(format))) {
      throw new CmdError({ code: 'invalid', message: 'the fix is not a valid parser change: `format` must be a table', retryable: false });
    }
    const summary = await this.imt.workspaceSummary();
    const wanted = fix.sources?.length ? new Set(fix.sources) : null;
    const eval_: FixEvaluation = { fixed: 0, checked: 0, unchecked: 0 };
    for (const s of summary.sources) {
      const issue = s.timestampIssue;
      if (!issue || (wanted && !wanted.has(s.id))) continue;
      const checked = Math.min(issue.unparsedLines, 1000);
      eval_.checked += checked;
      eval_.unchecked += issue.unparsedLines - checked;
      if (issue.suggestion && fix.parserOverride.includes(issue.suggestion.format)) eval_.fixed += checked;
    }
    if (eval_.checked === 0) eval_.notes = ['no stored events have unreadable timestamps in these sources (skipped lines are not stored)'];
    return eval_;
  }

  async indexStatus(): Promise<IndexStatus> {
    const summary = await this.imt.workspaceSummary();
    return { indexed: summary.total, total: summary.total, ready: true };
  }
}
