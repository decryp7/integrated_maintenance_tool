import { HomeMarker, HomeRequest, HomeSummary } from '../../core/api.types';
import { AnalysisBackend } from '../../core/analysis-backend';
import { ImtBackend } from '../../core/backend';
import { HomeBackend } from '../home-backend';

/** Most markers and anomalies the engine sends for the history. */
const MAX_MARKS = 200;

/**
 * Plausible Home summaries for browser development and UI tests, composed from the mock's own summary, timeline and
 * analyses like the engine composes its store queries. It mimics the contract only: the Rust engine is the reference.
 */
export class MockHomeBackend extends HomeBackend {
  constructor(
    private readonly imt: ImtBackend,
    /** Resolved only when the analyses are asked for, so a quick summary works where no analysis backend is provided. */
    private readonly analysis: () => AnalysisBackend,
  ) {
    super();
  }

  async querySummary(request: HomeRequest): Promise<HomeSummary> {
    const summary = await this.imt.workspaceSummary();
    const levels = summary.levels;
    const analyses = request.analyses ?? true;
    const none = { result: [], coverage: { matched: 0, analysed: 0, limits: [] } };
    const [anomalies, groups] = analyses ? await Promise.all([this.analysis().queryAnomalies(request.query), this.analysis().queryExceptions(request.query)]) : [none, none];
    const timed = summary.from !== null && summary.to !== null;
    const timeline = timed ? await this.imt.queryTimeline({ query: request.query, buckets: Math.min(240, Math.max(10, request.buckets)), alwaysShowLifecycle: true }) : null;
    const histogram = timeline?.histogram ?? [];
    const merged = new Map<string, HomeMarker>();
    for (const m of timeline?.lanes.flatMap((l) => l.markers) ?? []) {
      const key = `${m.bucket}:${m.kind}`;
      const known = merged.get(key);
      if (known) merged.set(key, { ...known, ts: Math.min(known.ts, m.ts), count: known.count + m.count });
      else merged.set(key, { kind: m.kind, ts: m.ts, count: m.count });
    }
    return {
      sources: summary.sources.length,
      events: summary.total,
      errors: (levels.Error ?? 0) + (levels.Fatal ?? 0),
      warnings: levels.Warning ?? 0,
      from: summary.from,
      to: summary.to,
      bucketMs: timeline?.bucketMs ?? 0,
      histogram,
      errorTrend: histogram.map((b) => (b[4] ?? 0) + (b[5] ?? 0)),
      warningTrend: histogram.map((b) => b[3] ?? 0),
      markers: [...merged.values()].sort((a, b) => a.ts - b.ts || a.kind.localeCompare(b.kind)).slice(0, MAX_MARKS),
      anomalyCount: anomalies.result.length,
      anomalies: anomalies.result
        .map((a) => ({ kind: a.kind, from: a.from, to: a.to, message: a.message }))
        .sort((a, b) => a.from - b.from || a.to - b.to)
        .slice(0, MAX_MARKS),
      exceptionGroups: groups.result.length,
      exceptionEvents: groups.result.reduce((n, g) => n + g.count, 0),
      analysed: analyses,
      coverage: {
        matched: anomalies.coverage.matched + groups.coverage.matched,
        analysed: anomalies.coverage.analysed + groups.coverage.analysed,
        limits: [...anomalies.coverage.limits, ...groups.coverage.limits],
      },
    };
  }
}
