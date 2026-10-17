import { MockFiles } from '../../core/mock/mock-import';
import {
  AgentConfigDoc,
  AgentConfigFile,
  AgentFieldSchema,
  AgentIssue,
  AgentSchema,
  RunManifest,
  RunsListing,
  RunSummary,
} from '../../core/agent.types';

/** Default config path of the mock (as the app's config folder would give). */
export const MOCK_AGENT_PATH = '/demo/config/imt-agent.toml';
export const MOCK_OUTPUT_DIR = '/demo/agent/runs';

const f = (key: string, label: string, kind: AgentFieldSchema['kind'], extra: Partial<AgentFieldSchema> = {}): AgentFieldSchema => ({
  key,
  label,
  kind,
  required: false,
  help: '',
  default: null,
  ...extra,
});

/** A copy of the agent's schema (crates/imt-agent/src/schema.rs) for browser development. */
export function mockAgentSchema(): AgentSchema {
  return {
    version: 1,
    agent: [
      f('outputDir', 'Output folder', 'path', { required: true, help: 'Each run gets a folder here, with a manifest.json.' }),
      f('stateDir', 'State folder', 'path', { help: 'Lock files and state.json. Default: <output>/.imt-agent.' }),
      f('logDir', 'Log folder', 'path', { help: "The agent's CLEF logs. Default: <state>/logs." }),
      f('minFreeMb', 'Minimum free space (MB)', 'number', { default: 1024, min: 0, max: 10_000_000 }),
      f('tickSecs', 'Check every (s)', 'number', { default: 15, min: 1, max: 3600 }),
      f('timezone', 'Cron time zone', 'select', { default: 'local', options: [{ value: 'local', label: 'Local time' }, { value: 'utc', label: 'UTC' }] }),
      f('logmanTimeoutSecs', 'logman timeout (s)', 'number', { default: 120, min: 5, max: 600, help: 'How long one logman call of a PerfMon capture (start, stop, query) may take (Windows).' }),
    ],
    log: [
      f('level', 'Log level', 'select', {
        default: 'information',
        options: ['debug', 'information', 'warning', 'error'].map((v) => ({ value: v, label: v[0].toUpperCase() + v.slice(1) })),
      }),
      f('maxFileMb', 'Roll at (MB)', 'number', { default: 10, min: 1, max: 1024 }),
      f('keepFiles', 'Files kept', 'number', { default: 5, min: 1, max: 100 }),
      f('maxAgeDays', 'Delete logs after (days)', 'number', { default: 30, min: 1, max: 3650 }),
    ],
    retention: [
      f('maxRuns', 'Keep at most (runs)', 'number', { default: 50, min: 1, max: 100_000, help: 'Empty: no limit.' }),
      f('maxTotalMb', 'Keep at most (MB)', 'number', { default: 5120, min: 1, max: 100_000_000, help: 'Empty: no limit.' }),
      f('maxAgeDays', 'Delete runs after (days)', 'number', { default: 30, min: 1, max: 36_500, help: 'Empty: no limit.' }),
    ],
    schedule: [f('name', 'Name', 'text', { required: true }), f('enabled', 'Enabled', 'boolean', { default: true })],
    timing: [
      { kind: 'cron', label: 'Cron expression', help: 'minute hour day month weekday, e.g. 0 2 * * *', fields: [f('cron', 'Cron', 'cron', { required: true, default: '0 2 * * *' })] },
      { kind: 'every', label: 'Fixed interval', help: 'Between 1m and 31d.', fields: [f('every', 'Every', 'duration', { required: true, default: '1h' })] },
      { kind: 'trigger', label: 'Trigger', help: 'Runs when a condition is met.', fields: [] },
    ],
    triggers: [
      {
        kind: 'process',
        label: 'Process threshold',
        help: 'Fires when a matching process stays above the threshold.',
        fields: [
          f('process', 'Process name', 'text', { required: true }),
          f('metric', 'Metric', 'select', {
            required: true,
            default: 'privateBytes',
            options: [
              { value: 'privateBytes', label: 'Private bytes (MB)' },
              { value: 'workingSet', label: 'Working set (MB)' },
              { value: 'cpu', label: 'CPU (% of the machine)' },
            ],
          }),
          f('above', 'Above', 'number', { required: true, min: 0 }),
          f('forSecs', 'For at least (s)', 'number', { default: 0, min: 0 }),
          f('cooldownSecs', 'Cooldown (s)', 'number', { default: 900, min: 0 }),
        ],
      },
      {
        kind: 'log',
        label: 'Log event',
        help: 'Fires when a new event in the watched log matches the query.',
        fields: [f('path', 'Log file or folder', 'path', { required: true }), f('ql', 'Query', 'query', { required: true })],
      },
    ],
    actions: [
      {
        kind: 'collect',
        label: 'Collect diagnostics',
        help: 'Runs the diagnostics collector into an .imtz bundle.',
        fields: [f('planFile', 'Plan file', 'path'), f('plan', 'Inline plan', 'json')],
      },
      {
        kind: 'monitor',
        label: 'Record process monitor',
        help: 'Samples processes for a while into CLEF files.',
        fields: [
          f('targets', 'Processes', 'list', { required: true }),
          f('duration', 'Duration', 'duration', { required: true, default: '5m' }),
          f('intervalMs', 'Interval (ms)', 'number', { default: 1000, min: 250 }),
          f('includeChildren', 'Include child processes', 'boolean', { default: false }),
        ],
      },
      {
        kind: 'copyLogs',
        label: 'Copy logs',
        help: 'Copies log files, newest first, within the size limit.',
        fields: [f('from', 'Folder or file', 'path', { required: true }), f('include', 'Include', 'list'), f('maxTotalMb', 'At most (MB)', 'number', { default: 200, min: 1 })],
      },
      {
        kind: 'perfmon',
        label: 'Capture PerfMon (Windows)',
        help: 'Starts an installed Data Collector Set and copies its logs.',
        fields: [f('collector', 'Collector set', 'text', { required: true }), f('duration', 'Duration', 'duration', { required: true, default: '5m' })],
      },
    ],
  };
}

export function mockStarter(): AgentConfigFile {
  return {
    version: 1,
    agent: { outputDir: MOCK_OUTPUT_DIR, minFreeMb: 1024, tickSecs: 15, timezone: 'local', logmanTimeoutSecs: 120, log: { level: 'information', maxFileMb: 10, keepFiles: 5, maxAgeDays: 30 } },
    retention: { maxRuns: 50, maxTotalMb: 5120, maxAgeDays: 30 },
    schedules: [{ name: 'nightly', enabled: true, cron: '0 2 * * *', actions: [{ kind: 'collect' }] }],
  };
}

const DURATION = /^\s*\d+\s*(ms|s|m|h|d)?\s*$/;
const NAME = /^[A-Za-z0-9 _.-]{1,64}$/;

/** A plausible subset of the agent's validation (the real rules are in Rust). */
export function mockValidate(cfg: AgentConfigFile): AgentIssue[] {
  const issues: AgentIssue[] = [];
  const bad = (path: string, message: string) => issues.push({ path, message });
  if (!cfg.agent.outputDir?.trim()) bad('agent.outputDir', 'a path is required');
  if (!(cfg.agent.tickSecs >= 1 && cfg.agent.tickSecs <= 3600)) bad('agent.tickSecs', 'must be between 1 and 3600');
  const logman = cfg.agent.logmanTimeoutSecs ?? 120;
  if (!(logman >= 5 && logman <= 600)) bad('agent.logmanTimeoutSecs', 'must be between 5 and 600');
  const names = new Set<string>();
  cfg.schedules.forEach((s, i) => {
    const key = `schedules[${i}]`;
    if (!NAME.test(s.name) || s.name.startsWith('.')) bad(`${key}.name`, "use 1-64 letters, digits, spaces, '-', '_' or '.'");
    if (names.has(s.name.toLowerCase())) bad(`${key}.name`, `another schedule is already called "${s.name}"`);
    names.add(s.name.toLowerCase());
    const set = [s.cron, s.every, s.trigger].filter((v) => v !== undefined).length;
    if (set !== 1) bad(key, 'set exactly one of "cron", "every" or "trigger"');
    if (s.cron !== undefined && !/^(@\w+|(\S+\s+){4,5}\S+)$/.test(s.cron.trim())) bad(`${key}.cron`, `invalid cron expression "${s.cron}"`);
    if (s.every !== undefined && !DURATION.test(s.every)) bad(`${key}.every`, `"${s.every}" is not a duration; use e.g. 30s, 15m, 2h or 1d`);
    if (s.trigger?.kind === 'process' && !s.trigger.process?.trim()) bad(`${key}.trigger.process`, 'a process name is required');
    if (s.trigger?.kind === 'log' && !s.trigger.ql?.trim()) bad(`${key}.trigger.ql`, 'a query is required, e.g. level >= Error');
    if (!s.actions.length) bad(`${key}.actions`, 'add at least one action');
    s.actions.forEach((a, j) => {
      const ak = `${key}.actions[${j}]`;
      if ((a.kind === 'monitor' || a.kind === 'perfmon') && !DURATION.test(a.duration ?? '')) bad(`${ak}.duration`, 'must be between 1s and 24h');
      if (a.kind === 'monitor' && !a.targets?.length) bad(`${ak}.targets`, 'config needs at least one entry in "targets"');
      if (a.kind === 'copyLogs' && !a.from?.trim()) bad(`${ak}.from`, 'a path is required');
      if (a.kind === 'perfmon' && !a.collector?.trim()) bad(`${ak}.collector`, 'enter the name of an installed Data Collector Set');
    });
  });
  return issues;
}

const H = 3_600_000;

function demoRuns(now: number): RunSummary[] {
  const run = (id: string, schedule: string, status: RunSummary['status'], hoursAgo: number, reason: string, error: string | null = null): RunSummary => ({
    id,
    dir: `${MOCK_OUTPUT_DIR}/${id}`,
    schedule,
    status,
    reason,
    startedMs: now - hoursAgo * H,
    durationMs: 42_000,
    totalBytes: 12_582_912,
    actions: 2,
    error,
  });
  return [
    run('20261017-020000-nightly', 'nightly', 'ok', 2, 'scheduled (At 02:00)'),
    run('20261016-143012-memory-spike', 'memory-spike', 'partial', 14, 'triggered: OrderService (4120) PrivateBytes = 1612.0MB > 1500MB', 'perfmon: only available on Windows'),
    run('20261016-020000-nightly', 'nightly', 'failed', 26, 'scheduled (At 02:00)', 'copyLogs: path does not exist'),
  ];
}

export interface MockSchedulesOptions {
  /** Start with a saved config (true) or no file yet (false). */
  saved: boolean;
  /** Time used for the demo runs. */
  now: number;
}

/** In-memory agent config and runs for the browser build and UI tests. */
export class MockSchedulesBackend {
  readonly calls: string[] = [];
  private saved: AgentConfigFile | null;
  private readonly runList: RunSummary[];
  /** Config files that exist (saving over one needs `overwrite`, like the engine). */
  private readonly files = new MockFiles();

  constructor(options: Partial<MockSchedulesOptions> = {}) {
    this.saved = options.saved === false ? null : mockStarter();
    if (this.saved) this.files.write(MOCK_AGENT_PATH, false);
    this.runList = demoRuns(options.now ?? Date.now());
  }

  async schema(): Promise<AgentSchema> {
    return mockAgentSchema();
  }

  async load(path?: string): Promise<AgentConfigDoc> {
    this.calls.push(`load:${path ?? ''}`);
    const config = structuredClone(this.saved ?? mockStarter());
    const file = path || MOCK_AGENT_PATH;
    return { path: file, exists: !!this.saved && this.files.has(file), config, issues: mockValidate(config) };
  }

  async validate(config: AgentConfigFile): Promise<AgentIssue[]> {
    this.calls.push('validate');
    return mockValidate(config);
  }

  async save(config: AgentConfigFile, path?: string, overwrite = false): Promise<AgentIssue[]> {
    this.calls.push(`save:${path ?? ''}${overwrite ? ':overwrite' : ''}`);
    const issues = mockValidate(config);
    if (issues.length) return issues;
    this.files.write(path || MOCK_AGENT_PATH, overwrite);
    this.saved = structuredClone(config);
    return issues;
  }

  async runs(): Promise<RunsListing> {
    this.calls.push('runs');
    if (!this.saved) throw `${MOCK_AGENT_PATH} does not exist yet; save the schedules first`;
    return { outputDir: this.saved.agent.outputDir, runs: structuredClone(this.runList), truncated: 0 };
  }

  async manifest(runId: string): Promise<RunManifest> {
    this.calls.push(`manifest:${runId}`);
    const r = this.runList.find((x) => x.id === runId);
    if (!r) throw `the run ${runId} no longer exists`;
    const failed = r.status !== 'ok';
    return {
      schema: 1,
      id: r.id,
      schedule: r.schedule,
      reason: r.reason.startsWith('triggered') ? { kind: 'trigger', detail: r.reason.slice(11) } : { kind: 'schedule', timer: 'At 02:00' },
      host: 'WEB01',
      agentVersion: '0.1.0',
      startedAt: new Date(r.startedMs).toISOString(),
      startedMs: r.startedMs,
      finishedAt: new Date(r.startedMs + r.durationMs).toISOString(),
      durationMs: r.durationMs,
      status: r.status,
      actions: [
        { kind: 'collect', status: 'ok', outputs: [`collect/imt-diagnostics-WEB01.imtz`], bytes: 10_485_760, ms: 30_000 },
        { kind: failed ? 'copyLogs' : 'monitor', status: failed ? 'failed' : 'ok', message: r.error, outputs: failed ? [] : ['monitor'], bytes: failed ? 0 : 2_097_152, ms: 12_000 },
      ],
      totalBytes: r.totalBytes,
      errors: [],
    };
  }

  async openRun(runId: string): Promise<string> {
    this.calls.push(`openRun:${runId}`);
    if (!this.runList.some((x) => x.id === runId)) throw `the run ${runId} no longer exists`;
    return 'window-2';
  }
}
