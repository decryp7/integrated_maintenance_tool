/**
 * The folder set-up for the browser build (dev and UI tests): a plausible survey of the mock incident and
 * proposals that work against the mock workspace (applying them changes its lanes and markers). It mimics
 * the contract only; the engine (`crates/imt-core/src/survey`) is the reference for behaviour.
 */
import { Injector } from '@angular/core';
import { ImtBackend } from '../../core/backend';
import { spanRules } from '../../core/mock/mock-runs';
import { EMPTY_QUERY } from '../../core/workspace.store';
import { parseProfile, stringifyProfile } from '../../profile/toml';
import { AiError } from '../ai-backend';
import { FolderSetupBackend } from './folder-setup-backend';
import { MockJobCancelled, mockJobs } from '../../jobs/mock-jobs';
import {
  ComposedProfile,
  DiffOp,
  FolderRefinement,
  FolderSetup,
  Fragment,
  LanePreview,
  SetupProposal,
  SurveyOutcome,
  SurveyProgress,
  SurveyStage,
} from './folder-setup.types';

const STAGES: SurveyStage[] = ['sampling', 'analysing', 'proposing', 'validating'];

export interface MockFolderSetupOptions {
  /** Delay per survey stage in ms (0 = answer at once, for unit tests). */
  stageMs?: number;
  /** The AI refinement rejects with this code (e.g. 'noModel'). */
  aiError?: 'noModel' | 'unavailable' | null;
}

const marker = (kind: string, label: string, contains: string, color: string, icon: string): Fragment => ({
  kind: 'marker',
  marker: { kind, label, lifecycle: true, color, icon, description: 'Found by the folder survey', when: [{ field: '@message', op: 'contains', value: contains }] },
});

function proposal(p: Omit<SetupProposal, 'toml' | 'requires' | 'origin'> & Partial<Pick<SetupProposal, 'requires' | 'origin'>>): SetupProposal {
  return { requires: [], origin: 'heuristic', ...p, toml: snippet(p.fragments) };
}

/** A TOML rendering of fragments like the engine's (for review only). */
export function snippet(fragments: Fragment[]): string {
  const model: Record<string, unknown[] | Record<string, unknown>> = {};
  const comments: string[] = [];
  const push = (key: string, v: unknown) => {
    const list = (model[key] ??= []) as unknown[];
    list.push(v);
  };
  for (const f of fragments) {
    switch (f.kind) {
      case 'parseRegex':
        model['parse'] = { regexes: [f.regex] };
        break;
      case 'instanceKey':
        model['instance'] = { key: f.key };
        break;
      case 'savedQuery':
        comments.push(`# Saved query "${f.name}": ${f.ql}`);
        break;
      case 'extractor':
        push('extract', f.extractor);
        break;
      case 'pipeline':
        push('pipeline', f.step);
        break;
      default:
        push(f.kind, (f as unknown as Record<string, unknown>)[f.kind]);
    }
  }
  const body = Object.keys(model).length ? stringifyProfile(model as never) : '';
  return [...comments, body].filter(Boolean).join('\n');
}

/** The mock incident's proposals; lane previews come from the mock workspace. */
export function mockSetup(lanes: LanePreview[]): FolderSetup {
  const instance = proposal({
    id: 'instance:MachineName',
    group: 'instances',
    title: 'One timeline lane per MachineName',
    summary: `Split the events into ${lanes.length || 4} lanes by the field MachineName (100% of the events have it), e.g. WEB01, WEB02, WORKER01.`,
    fragments: [{ kind: 'instanceKey', key: ['MachineName'] }],
    preview: { kind: 'lanes', lanes, totalLanes: lanes.length, unassigned: 0 },
    recommended: true,
  });
  const proposals: SetupProposal[] = [
    instance,
    proposal({
      id: 'instance:Application',
      group: 'instances',
      title: 'One timeline lane per Application',
      summary: 'Split the events into 3 lanes by the field Application (100% of the events have it), e.g. OrderService, OrderService.Worker, Gateway.',
      fragments: [{ kind: 'instanceKey', key: ['Application'] }],
      preview: {
        kind: 'lanes',
        lanes: [
          { name: 'OrderService', count: 6120, from: null, to: null },
          { name: 'OrderService.Worker', count: 2210, from: null, to: null },
          { name: 'Gateway', count: 1490, from: null, to: null },
        ],
        totalLanes: 3,
        unassigned: 0,
      },
      recommended: false,
    }),
    proposal({
      id: 'marker:reindex-start',
      group: 'markers',
      title: 'Mark “Nightly reindex started for {IndexCount} indexes” as service start',
      summary: '1 event(s) read like a service start and no marker shows them yet. Example: Nightly reindex started for 4 indexes',
      fragments: [marker('reindex-start', 'Reindex start', 'Nightly reindex started', '#3FBF7F', 'rocket')],
      preview: { kind: 'matches', matches: 1, sampled: 9820, examples: ['Nightly reindex started for 4 indexes'] },
      recommended: true,
    }),
    proposal({
      id: 'marker:reindex-stop',
      group: 'markers',
      title: 'Mark “Nightly reindex completed in {Elapsed}” as service stop',
      summary: '1 event(s) read like a service stop and no marker shows them yet. Example: Nightly reindex completed in 00:44:12',
      fragments: [marker('reindex-stop', 'Reindex stop', 'Nightly reindex completed', '#A7B0C2', 'power')],
      preview: { kind: 'matches', matches: 1, sampled: 9820, examples: ['Nightly reindex completed in 00:44:12'] },
      recommended: true,
    }),
    proposal({
      id: 'span:run',
      group: 'markers',
      title: 'Let the “Running” runs start and stop on the new markers',
      summary: 'Runs now start at start, reindex-start and stop at stop, reindex-stop; a start without a stop is flagged as a likely crash or restart.',
      fragments: [{ kind: 'span', span: { kind: 'run', label: 'Running', start: ['start', 'reindex-start'], stop: ['stop', 'reindex-stop'], per: 'instance' } }],
      requires: ['marker:reindex-start', 'marker:reindex-stop'],
      preview: { kind: 'runs', runs: 6, closed: 2, open: 3, noStop: 1, lanes: 4 },
      recommended: true,
    }),
    proposal({
      id: 'extract:Documents',
      group: 'extractors',
      title: 'Extract Documents as a number',
      summary: '“documents” follows a number in 12 sampled messages (from 120000 to 480000). Example: Reindexed orders (240000 documents) in 812 ms',
      fragments: [{ kind: 'extractor', extractor: { name: 'metric-documents', from: '@message', regex: '\\((?P<Documents>\\d+) documents\\)', fields: { Documents: { type: 'number' } } } }],
      preview: { kind: 'extracted', matches: 12, sampled: 9820, examples: [{ Documents: 240000 }, { Documents: 120000 }] },
      recommended: true,
    }),
    proposal({
      id: 'noise:0',
      group: 'noise',
      title: 'Drop “Cache {Outcome} for {CacheKey}”',
      summary: '1630 of the sampled events (17%) are this message and it looks like routine cache traffic.',
      fragments: [{ kind: 'pipeline', step: { step: 'drop', when: [{ field: '@message', op: 'contains', value: 'Cache hit for' }] } }],
      preview: { kind: 'pipeline', sampled: 9820, kept: 8190, dropped: 1630, changed: 0, examples: [{ before: 'Cache hit for cart:8812', after: '' }] },
      recommended: false,
    }),
    proposal({
      id: 'chart:errors-by-instance',
      group: 'charts',
      title: 'Chart errors per MachineName',
      summary: 'Errors and fatal events per interval, one series per MachineName, to see which instance fails when.',
      fragments: [{ kind: 'chart', chart: { id: 'errors-by-instance', title: 'Errors per MachineName', y: 'count()', bucket: 'auto', split_by: ['MachineName'], type: 'stacked', unit: 'events' } }],
      requires: ['instance:MachineName'],
      preview: { kind: 'chart', series: ['WEB02', 'GATEWAY', 'WEB01', 'WORKER01'], points: 214 },
      recommended: true,
    }),
    proposal({
      id: 'dashboard:folder-overview',
      group: 'charts',
      title: 'Add a starter dashboard',
      summary: 'Error and warning counts, events by level, the busiest MachineName values and the most frequent errors on one page.',
      fragments: [{ kind: 'dashboard', dashboard: { id: 'folder-overview', title: 'Folder overview', columns: 12, tile: [{ id: 'errors', title: 'Errors', type: 'stat', query: 'level >= error | stats count()', w: 3, h: 2 }] } }],
      preview: { kind: 'dashboard', tiles: 5 },
      recommended: false,
    }),
    proposal({
      id: 'query:error:0',
      group: 'queries',
      title: 'Save a query for “Failed to capture payment for order {OrderId}: request timed out”',
      summary: '96 sampled error events like: Failed to capture payment for order 100412: request timed out',
      fragments: [{ kind: 'savedQuery', name: 'Errors: Failed to capture payment for order {OrderId}…', ql: 'message contains "Failed to capture payment"' }],
      preview: { kind: 'query', matches: 182 },
      recommended: true,
    }),
  ];
  return {
    survey: {
      totalEvents: 9820,
      sampledEvents: 9820,
      truncated: false,
      elapsedMs: 412,
      groups: [
        { id: 'json:web01/orders-20261014.clef', importer: 'json', name: 'web01/orders-20261014.clef', files: 1, events: 3120, sampled: 2000, from: null, to: null },
        { id: 'json:web02/orders-20261014.clef', importer: 'json', name: 'web02/orders-20261014.clef', files: 1, events: 3000, sampled: 2000, from: null, to: null },
        { id: 'text:worker01/worker-20261014.log', importer: 'text', name: 'worker01/worker-20261014.log', files: 1, events: 2210, sampled: 2000, from: null, to: null },
        { id: 'json:gateway/gateway-20261014.json', importer: 'json', name: 'gateway/gateway-20261014.json', files: 1, events: 1490, sampled: 1490, from: null, to: null },
      ],
      unrecognised: [
        { extension: 'reg', fileCount: 1, files: ['system/registry-export.reg'], reason: 'no importer recognises this format', sampleFile: 'system/registry-export.reg', sampleLines: ['Windows Registry Editor Version 5.00'] },
      ],
      fields: [],
      instanceCandidates: [{ field: 'MachineName', origin: { kind: 'field' }, lanes: lanes.length || 4, coverage: 1, score: 6.1, examples: lanes.map((l) => l.name) }],
      lifecycle: [
        { role: 'start', template: 'Application starting — {Application} {Version} ({Framework})', templateKind: 'serilog', count: 6, example: 'Application starting — OrderService 3.14.2 (net9.0)', coveredBy: 'start' },
        { role: 'start', template: 'Nightly reindex started for {IndexCount} indexes', templateKind: 'serilog', count: 1, example: 'Nightly reindex started for 4 indexes', coveredBy: null },
      ],
      errors: [{ template: 'Failed to capture payment for order {OrderId}: request timed out', templateKind: 'serilog', count: 96, level: 'Error', share: 0.01, example: 'Failed to capture payment for order 100412: request timed out' }],
      noise: [],
      numbers: [],
      markers: [],
    },
    proposals,
    skipped: [{ id: 'format:reg:0', group: 'formats', origin: 'heuristic', title: 'Read .reg files as text with a day-first or US date', reason: 'it reads 0 of 12 sample lines with a timestamp (0%)' }],
  };
}

/** The AI's additions in the mock: a payment-timeout marker and a saved query. */
export function mockRefinement(): FolderRefinement {
  return {
    proposals: [
      proposal({
        id: 'ai:marker:payment-timeout',
        group: 'markers',
        origin: 'ai',
        title: 'Mark Payment timeout',
        summary: 'The payment gateway did not answer in time (14 of 120 events in the model’s sample)',
        fragments: [marker('payment-timeout', 'Payment timeout', 'Payment gateway call exceeded', '#E9AE3F', 'timer')],
        preview: { kind: 'matches', matches: 41, sampled: 9820, examples: ['Payment gateway call exceeded 5000 ms (attempt 3/3) for order 100412'] },
        recommended: false,
      }),
    ],
    skipped: [{ id: 'ai:extract:attempt', group: 'extractors', origin: 'ai', title: 'Extract attempt', reason: 'extractor “attempt” extracts nothing from the 9820 sampled events' }],
    notes: [],
  };
}

/** A naive line diff (common prefix and suffix, the rest removed then added): enough for the mock. */
export function naiveDiff(before: string, after: string): { op: DiffOp; text: string }[] {
  const a = before.split('\n');
  const b = after.split('\n');
  let p = 0;
  while (p < a.length && p < b.length && a[p] === b[p]) p++;
  let s = 0;
  while (s < a.length - p && s < b.length - p && a[a.length - 1 - s] === b[b.length - 1 - s]) s++;
  return [
    ...a.slice(0, p).map((text) => ({ op: 'same' as const, text })),
    ...a.slice(p, a.length - s).map((text) => ({ op: 'remove' as const, text })),
    ...b.slice(p, b.length - s).map((text) => ({ op: 'add' as const, text })),
    ...a.slice(a.length - s).map((text) => ({ op: 'same' as const, text })),
  ];
}

export class MockFolderSetupBackend extends FolderSetupBackend {

  constructor(
    private readonly workspace: ImtBackend,
    private readonly options: MockFolderSetupOptions = {},
  ) {
    super();
  }

  private wait(ms: number): Promise<void> {
    return ms > 0 ? new Promise((r) => setTimeout(r, ms)) : Promise.resolve();
  }

  /** Like the engine: an analysis job, returned at once; its result is the survey's outcome. */
  async survey(onProgress: (p: SurveyProgress) => void): Promise<number> {
    const job = mockJobs().start({ kind: 'analysis', title: 'Survey the folder' }, async (ctx) => {
      const summary = await this.workspace.workspaceSummary();
      if (!summary.sources.length) throw new Error('nothing is open: open a folder first');
      for (const [i, stage] of STAGES.entries()) {
        onProgress({ stage, done: i, total: STAGES.length });
        ctx.progress({ done: i, total: STAGES.length, fraction: i / STAGES.length });
        await this.wait(this.options.stageMs ?? 150);
        if (ctx.cancelled()) throw new MockJobCancelled();
      }
      onProgress({ stage: 'done', done: STAGES.length, total: STAGES.length });
      const outcome: SurveyOutcome = { setup: mockSetup(await this.lanes()) };
      return { value: outcome, result: { type: 'folderSurvey', outcome } };
    });
    job.done.catch(() => undefined);
    return job.id;
  }

  private async lanes(): Promise<LanePreview[]> {
    try {
      const facet = await this.workspace.queryFacet({ query: EMPTY_QUERY, field: 'MachineName', limit: 12 });
      return facet.values.map((v) => ({ name: v.value, count: v.count, from: null, to: null }));
    } catch {
      return [];
    }
  }

  async compose(fragments: Fragment[]): Promise<ComposedProfile> {
    if (!fragments.length) throw new Error('select at least one proposal');
    const before = (await this.workspace.getProfile()).toml;
    const model = parseProfile(before) as unknown as Record<string, unknown>;
    const list = (key: string) => ((model[key] as unknown[] | undefined) ??= []) as unknown[];
    const savedQueries: { name: string; ql: string }[] = [];
    const newMarkers: unknown[] = [];
    for (const f of fragments) {
      switch (f.kind) {
        case 'instanceKey':
          model['instance'] = { ...((model['instance'] as object) ?? {}), key: f.key };
          break;
        case 'parseRegex': {
          const parse = ((model['parse'] as Record<string, unknown>) ??= {});
          parse['regexes'] = [...((parse['regexes'] as string[]) ?? []), f.regex];
          break;
        }
        case 'savedQuery':
          savedQueries.push({ name: f.name, ql: f.ql });
          break;
        case 'marker':
          if ((model['marker'] as { kind?: string }[] | undefined)?.some((m) => m.kind === f.marker['kind'])) {
            throw new Error(`the profile already has a marker named "${String(f.marker['kind'])}"; deselect one of them`);
          }
          newMarkers.push(f.marker);
          break;
        case 'extractor':
          list('extract').push(f.extractor);
          break;
        case 'pipeline':
          list('pipeline').push(f.step);
          break;
        case 'chart':
          list('chart').push(f.chart);
          break;
        case 'dashboard':
          list('dashboard').push(f.dashboard);
          break;
        case 'span': {
          // Like the engine: the default rule is kept, a rule of the same kind is replaced in place.
          const spans = (model['span'] ??= spanRules(model as never).map((s) => ({ ...s }))) as { kind?: unknown }[];
          const at = spans.findIndex((s) => s.kind === f.span['kind']);
          if (at >= 0) spans[at] = f.span;
          else spans.push(f.span);
          break;
        }
      }
    }
    if (newMarkers.length) model['marker'] = [...newMarkers, ...((model['marker'] as unknown[]) ?? [])];
    const normalisedBefore = stringifyProfile(parseProfile(before));
    const toml = stringifyProfile(model as never);
    const diff = naiveDiff(normalisedBefore, toml);
    return { toml, diff, added: diff.filter((d) => d.op === 'add').length, removed: diff.filter((d) => d.op === 'remove').length, savedQueries };
  }

  /** Like the engine: an AI job, returned at once; its result is a `folderSetup` proposal. */
  async refineWithAi(): Promise<number> {
    if (this.options.aiError) throw new AiError(this.options.aiError, this.options.aiError === 'noModel' ? 'No AI model is set up yet.' : 'The AI assistant is not available.');
    const job = mockJobs().start({ kind: 'ai', title: 'Improve the folder set-up' }, async (ctx) => {
      await this.wait((this.options.stageMs ?? 150) * 2);
      if (ctx.cancelled()) throw new MockJobCancelled();
      const refinement: FolderRefinement = mockRefinement();
      return { value: refinement, result: { type: 'proposal', proposal: { ...refinement, kind: 'folderSetup' } } };
    });
    job.done.catch(() => undefined);
    return job.id;
  }
}

export function createMockFolderSetupBackend(injector: Injector, options?: MockFolderSetupOptions): FolderSetupBackend {
  return new MockFolderSetupBackend(injector.get(ImtBackend), options);
}
