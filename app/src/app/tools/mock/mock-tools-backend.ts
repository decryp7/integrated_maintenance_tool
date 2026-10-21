import {
  Manifest,
  AppStatus,
  CollectorAction,
  CollectorStatus,
  CollectorTemplate,
  CollectPlan,
  CollectResult,
  CounterPreset,
  ElevationInfo,
  EventLogChannel,
  EventLogQuery,
  EventLogStartArgs,
  EventLogStarted,
  EventLogStatus,
  EventLogValidation,
  GeneratedCollector,
  MonitorConfig,
  MonitorStatus,
  Platform,
  ProjectOpened,
  ProjectSaved,
  TOOL_IDS,
  ToolAvailability,
  ToolEntry,
  ToolSchema,
  TOOLS_PLUGIN,
} from '../../core/tools.types';
import type { Report } from '../../core/api.types';
import type { ToolsBackend, Unlisten } from '../../core/tools-backend';
import { MockFiles } from '../../core/mock/mock-import';
import { DEFAULT_PLAN } from '../collector/plan-model';
import { MockEventLog } from './mock-event-log';

/** Switches for e2e tests and browser development, read from the page URL (`?mock-crash=1`). */
export interface MockToolsOptions {
  safeMode: boolean;
  previousCrash: boolean;
  elevated: boolean;
  /** The operating system to pretend to run on (PerfMon management and elevation are Windows only). */
  platform: Platform;
  /** Management commands fail with "Access is denied". */
  denied: boolean;
  /** Delay for slow operations such as the collector, in ms. */
  latency: number;
  /** State of the tools plugin: its tools are listed with the reason, or (disabled) not at all. */
  toolsPlugin: MockToolsPluginState;
}

/** How the mock's tools plugin is doing (`?mock-tools=quarantined`). */
export type MockToolsPluginState = 'active' | 'quarantined' | 'inactive' | 'disabled';

export const MOCK_DEFAULTS: MockToolsOptions = {
  safeMode: false,
  previousCrash: false,
  elevated: false,
  platform: 'windows',
  denied: false,
  latency: 0,
  toolsPlugin: 'active',
};

const MOCK_TOOLS_STATES: readonly MockToolsPluginState[] = [
  'active',
  'quarantined',
  'inactive',
  'disabled',
];

const MOCK_OS: Record<string, Platform> = { mac: 'macos', macos: 'macos', linux: 'linux' };

/** Reads the `mock-*` query parameters (`mock-os=mac` or `mock-os=linux` turns Windows off). */
export function optionsFromSearch(search: string): MockToolsOptions {
  const q = new URLSearchParams(search);
  const flag = (k: string) => q.get(k) === '1';
  return {
    safeMode: flag('mock-safe'),
    previousCrash: flag('mock-crash'),
    elevated: flag('mock-elevated'),
    platform: MOCK_OS[q.get('mock-os') ?? ''] ?? 'windows',
    denied: flag('mock-denied'),
    latency: Number(q.get('mock-latency') ?? 0) || 0,
    toolsPlugin: MOCK_TOOLS_STATES.find((s) => s === q.get('mock-tools')) ?? 'active',
  };
}

const PLUGIN_REASONS: Record<'quarantined' | 'inactive', string> = {
  quarantined: `plugin "${TOOLS_PLUGIN}" is disabled after repeated crashes (re-enable it in the Plugins panel)`,
  inactive: `plugin "${TOOLS_PLUGIN}" is inactive: needs net.decryptology.windows >=0.1 (not installed)`,
};

const ANY_ARGS: ToolSchema = { type: 'object', fields: [], additional: false };

/** The tools the engine's tools plugin contributes, as `tools_list` reports them (availability added per call). */
const MOCK_TOOLS: Omit<ToolEntry, 'availability'>[] = [
  {
    id: TOOL_IDS.perfmonTemplates,
    title: 'Performance Monitor templates',
    description: 'Counter presets and Data Collector Set XML.',
    platforms: [],
    needsElevation: false,
    ops: [{ name: 'presets', title: 'Counter presets', args: ANY_ARGS, budgetMs: 5000 }],
    plugin: TOOLS_PLUGIN,
  },
  {
    id: TOOL_IDS.perfmonSets,
    title: 'Performance Monitor collector sets',
    description: 'Creates and controls Data Collector Sets with logman.',
    platforms: ['windows'],
    needsElevation: true,
    ops: [{ name: 'outputs', title: 'Log files', args: ANY_ARGS, budgetMs: 900_000 }],
    plugin: TOOLS_PLUGIN,
  },
  {
    id: TOOL_IDS.processMonitor,
    title: 'Process monitor',
    description: 'Samples processes into rolling CLEF files.',
    platforms: [],
    needsElevation: false,
    ops: [{ name: 'status', title: 'Whether it runs', args: ANY_ARGS, budgetMs: 5000 }],
    plugin: TOOLS_PLUGIN,
  },
  {
    id: TOOL_IDS.diagnostics,
    title: 'Diagnostics collector',
    description: 'Gathers logs and system information into a bundle.',
    platforms: [],
    needsElevation: false,
    ops: [{ name: 'default-plan', title: 'The default plan', args: ANY_ARGS, budgetMs: 5000 }],
    plugin: TOOLS_PLUGIN,
  },
  {
    id: TOOL_IDS.elevation,
    title: 'Administrator rights',
    description: 'Whether IMT runs elevated.',
    platforms: [],
    needsElevation: false,
    ops: [{ name: 'info', title: 'Elevation and platform', args: ANY_ARGS, budgetMs: 10_000 }],
    plugin: TOOLS_PLUGIN,
  },
  {
    id: TOOL_IDS.eventLog,
    title: 'Event Log follower',
    description: 'Follows Windows event log channels into rolling CLEF files that open with live tail.',
    platforms: ['windows'],
    needsElevation: false,
    ops: ['channels', 'validate', 'start', 'stop', 'pause', 'resume', 'status'].map((name) => ({
      name,
      title: name,
      args: ANY_ARGS,
      budgetMs: 60_000,
    })),
    plugin: TOOLS_PLUGIN,
  },
];

const PLATFORM_LABEL: Record<Platform, string> = {
  windows: 'Windows',
  macos: 'macOS',
  linux: 'Linux',
};

const PRESETS: CounterPreset[] = [
  {
    id: 'cpu',
    name: 'CPU',
    description: 'Per-process and machine CPU use, run queue and context switches.',
    counters: [
      {
        path: '\\Process({process})\\% Processor Time',
        guidance: 'Sustained near 100% per core means the process is CPU bound.',
      },
      {
        path: '\\Process({process})\\Thread Count',
        guidance: 'A steady climb signals a thread leak.',
      },
      {
        path: '\\System\\Processor Queue Length',
        guidance: 'More than 2 per core for long stretches means threads wait for a CPU.',
      },
    ],
  },
  {
    id: 'memory',
    name: 'Memory',
    description: 'Per-process memory, handles and pools; finds leaks.',
    counters: [
      {
        path: '\\Process({process})\\Private Bytes',
        guidance: 'A line that only rises, never returning after load, is a memory leak.',
      },
      {
        path: '\\Process({process})\\Handle Count',
        guidance: 'Continuous growth means leaked handles.',
      },
      { path: '\\Memory\\Available MBytes', guidance: null },
    ],
  },
  {
    id: 'disk',
    name: 'Disk',
    description: 'Per-process I/O volume and physical disk latency.',
    counters: [
      {
        path: '\\PhysicalDisk(_Total)\\Avg. Disk sec/Read',
        guidance: 'Above 20 ms sustained is a slow disk.',
      },
    ],
  },
];

const FORMAT_CODE = { csv: '0', tsv: '1', blg: '3' } as const;

function escapeXml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

/** A plausible Data Collector Set document (the real one is produced by the Rust engine). */
export function mockXml(t: CollectorTemplate): string {
  const all = PRESETS.filter((p) => t.presets.includes(p.id)).flatMap((p) =>
    p.counters.map((c) => c.path),
  );
  const paths = [...all, ...t.extraCounters];
  const targets = t.processes.length ? t.processes : ['*'];
  const counters = targets.flatMap((proc) =>
    paths.map((p) => `    <Counter>${escapeXml(p.replace('{process}', proc))}</Counter>`),
  );
  return [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<DataCollectorSet>',
    `  <Name>${escapeXml(t.name)}</Name>`,
    `  <RootPath>${escapeXml(t.outputDir)}</RootPath>`,
    '  <PerformanceCounterDataCollector>',
    `    <LogFileFormat>${FORMAT_CODE[t.format]}</LogFileFormat>`,
    `    <SampleInterval>${t.sampleIntervalSecs}</SampleInterval>`,
    ...counters,
    '  </PerformanceCounterDataCollector>',
    '</DataCollectorSet>',
  ].join('\n');
}

export function mockManifest(plan: CollectPlan): Manifest {
  return {
    name: plan.name,
    host: 'DEMO-HOST',
    platform: 'windows',
    createdAt: '2026-10-17T10:00:00Z',
    items: [
      {
        path: 'logs/app.log',
        kind: 'log',
        origin: plan.logs[0]?.path ?? 'C:\\logs',
        originalBytes: 52_000,
        bytes: 51_200,
        redacted: true,
        redactions: 3,
      },
      {
        path: 'environment.json',
        kind: 'environment',
        origin: 'environment',
        originalBytes: 2_100,
        bytes: 2_100,
        redacted: false,
        redactions: 0,
      },
    ],
    skipped: plan.msinfo
      ? []
      : [{ kind: 'msinfo', what: 'msinfo32', reason: 'disabled in the plan' }],
    redaction: { email: 2, token: 1 },
    errors: [],
    totalBytes: 53_300,
    durationMs: 420,
  };
}

/** Clock offsets (ms) by source path, as kept in a project. */
type ClockByPath = Record<string, number>;

/** In-memory stand-in for the tools commands: plausible behaviour for browser development and UI tests. */
export class MockToolsBackend implements ToolsBackend {
  private readonly sets = new Map<string, CollectorStatus>();
  private monitor: MonitorStatus = { running: false, dir: null };
  private session: unknown = null;
  private sessionReports: Report[] = [];
  private sessionClock: ClockByPath = {};
  private readonly projects = new Map<
    string,
    { name: string; ui: unknown; reports?: Report[]; clock?: ClockByPath }
  >();
  /** Files "written": projects, PerfMon XML and bundles (writing over one needs `overwrite`). */
  private readonly files = new MockFiles();
  private readonly disabled = new Set<string>();
  private monitorCount = 0;
  /**
   * The config of the current or last monitor run: the mock live charts write .NET runtime samples only when it
   * asked for them (`dotnet`), like the engine.
   */
  monitorConfig: MonitorConfig | null = null;
  /** Everything the UI asked to be done, for assertions. */
  readonly calls: string[] = [];
  /** The Event Log follower: Security needs `elevated`; `denied` refuses every start. */
  private readonly eventLog = new MockEventLog(() => ({
    elevated: this.options.elevated,
    denied: this.options.denied,
  }));

  constructor(
    readonly options: MockToolsOptions = optionsFromSearch(
      typeof location === 'undefined' ? '' : location.search,
    ),
    private readonly summaryOf: () => Promise<ProjectOpened['summary']> = () =>
      Promise.reject(new Error('no workspace summary available')),
    /** Applies a source's clock offset in the mock workspace; projects keep offsets per source path like the engine's `clock`. */
    private readonly setOffset?: (source: number, offsetMs: number) => Promise<unknown>,
  ) {}

  /** Non-zero clock offsets of the open sources, by path (an `.imtproj` version 2 `clock`). */
  private async clockOf(): Promise<ClockByPath> {
    try {
      const summary = await this.summaryOf();
      return Object.fromEntries(
        summary.sources.filter((s) => s.offsetMs).map((s) => [s.path, s.offsetMs ?? 0]),
      );
    } catch {
      return {};
    }
  }

  /** Re-applies saved offsets to the sources with those paths (others keep theirs), then returns the summary. */
  private async restoreClock(clock: ClockByPath): Promise<ProjectOpened['summary']> {
    const summary = await this.summaryOf();
    if (!this.setOffset) return summary;
    const changes = summary.sources.filter((s) => (clock[s.path] ?? 0) !== (s.offsetMs ?? 0));
    for (const s of changes) await this.setOffset(s.id, clock[s.path] ?? 0);
    return changes.length ? this.summaryOf() : summary;
  }

  async toolsList(): Promise<ToolEntry[]> {
    const plugin = this.options.toolsPlugin;
    if (this.options.safeMode || plugin === 'disabled') return [];
    return MOCK_TOOLS.map((t) => ({ ...structuredClone(t), availability: this.availability(t) }));
  }

  private availability(t: Omit<ToolEntry, 'availability'>): ToolAvailability {
    const plugin = this.options.toolsPlugin;
    if (t.platforms.length && !t.platforms.includes(this.options.platform)) {
      return {
        state: 'unsupported',
        reason: `${t.title} is only available on Windows (this is ${PLATFORM_LABEL[this.options.platform]})`,
      };
    }
    return plugin === 'quarantined' || plugin === 'inactive'
      ? { state: 'pluginUnavailable', reason: PLUGIN_REASONS[plugin] }
      : { state: 'available' };
  }

  /** Status polls run the same operations (the real shell refuses ops not declared pollable). */
  toolPoll<T = unknown>(tool: string, op: string, args: unknown = {}): Promise<T> {
    return this.toolInvoke<T>(tool, op, args);
  }

  /** The mock lists no tool tasks as jobs. */
  async toolTaskJob(_task: string): Promise<number | null> {
    return null;
  }

  async toolInvoke<T = unknown>(tool: string, op: string, args: unknown = {}): Promise<T> {
    const entry = (await this.toolsList()).find((t) => t.id === tool);
    if (!entry) throw new Error(`no installed plugin provides the tool "${tool}"`);
    if (entry.availability.state !== 'available') throw new Error(entry.availability.reason);
    const log = TOOL_IDS.eventLog;
    const ops: Record<string, () => Promise<unknown>> = {
      [`${TOOL_IDS.perfmonTemplates}/presets`]: () => this.perfmonPresets(),
      [`${TOOL_IDS.processMonitor}/status`]: () => this.monitorStatus(),
      [`${TOOL_IDS.diagnostics}/default-plan`]: () => this.collectDefaultPlan(),
      [`${TOOL_IDS.elevation}/info`]: () => this.elevation(),
      [`${log}/channels`]: () => this.eventLog.channels(),
      [`${log}/validate`]: () => this.eventLog.validate(args as EventLogQuery),
      [`${log}/start`]: () => this.startEventLog(args as EventLogStartArgs),
      [`${log}/stop`]: () => this.eventLog.stop(),
      [`${log}/pause`]: () => this.eventLog.pause(),
      [`${log}/resume`]: () => this.eventLog.resume(),
      [`${log}/status`]: () => this.eventLog.status(),
    };
    const run = ops[`${tool}/${op}`];
    if (!run) throw new Error(`tool "${tool}" has no operation "${op}"`);
    return (await run()) as T;
  }

  private async slow<T>(value: T): Promise<T> {
    if (this.options.latency) await new Promise((r) => setTimeout(r, this.options.latency));
    return value;
  }

  private requireAdmin(): void {
    if (this.options.denied && !this.options.elevated)
      throw new Error('Access is denied. Administrator rights are required.');
  }

  private requireWindows(): void {
    if (this.options.platform !== 'windows')
      throw new Error('Performance Monitor collector sets can only be managed on Windows');
  }

  async perfmonPresets(): Promise<CounterPreset[]> {
    return structuredClone(PRESETS);
  }

  async perfmonGenerate(
    template: CollectorTemplate,
    xmlPath?: string,
    overwrite?: boolean,
  ): Promise<GeneratedCollector> {
    this.validate(template);
    const path = xmlPath ?? `${template.name}.xml`;
    // Like the engine: written (and guarded) only when a path is given.
    if (xmlPath) this.files.write(xmlPath, overwrite);
    return {
      xml: mockXml(template),
      path,
      commands: [
        `logman import -n "${template.name}" -xml "${path}"`,
        `logman start -n "${template.name}"`,
        `logman stop -n "${template.name}"`,
        `logman delete -n "${template.name}"`,
      ],
    };
  }

  async perfmonInstall(template: CollectorTemplate): Promise<CollectorStatus> {
    this.requireWindows();
    this.requireAdmin();
    this.validate(template);
    const status: CollectorStatus = {
      name: template.name,
      status: 'stopped',
      rootPath: template.outputDir,
      outputLocation: `${template.outputDir}${template.name}`,
      segment: false,
    };
    this.sets.set(template.name, status);
    return { ...status };
  }

  async perfmonControl(name: string, action: CollectorAction): Promise<CollectorStatus | null> {
    this.requireWindows();
    const set = this.sets.get(name);
    if (!set) throw new Error(`Data Collector Set "${name}" was not found`);
    if (action === 'delete') {
      this.sets.delete(name);
      return null;
    }
    if (action === 'start' || action === 'stop') {
      this.requireAdmin();
      set.status = action === 'start' ? 'running' : 'stopped';
    }
    return { ...set };
  }

  async perfmonOutputs(name: string): Promise<string[]> {
    const set = this.sets.get(name);
    if (!set) throw new Error(`Data Collector Set "${name}" was not found`);
    return [`${set.outputLocation}\\${name}_000001.csv`];
  }

  private validate(t: CollectorTemplate): void {
    if (!t.name.trim()) throw new Error('the collector name must be 1-100 characters');
    if (t.sampleIntervalSecs < 1)
      throw new Error('the sample interval must be between 1 and 86400 seconds');
    if (t.maxSizeMb != null && t.format !== 'blg')
      throw new Error('a circular log (maximum size) needs the binary BLG format');
  }

  async monitorStart(config: MonitorConfig, dir?: string): Promise<string> {
    if (this.monitor.running) throw new Error('the process monitor is already running');
    if (!config.targets.length) throw new Error('invalid monitor config: add at least one target');
    const folder = dir ?? `/demo/monitor/run-${++this.monitorCount}`;
    this.monitor = { running: true, dir: folder };
    this.monitorConfig = config;
    this.calls.push(`monitorStart:${folder}`);
    return folder;
  }

  async monitorStop(): Promise<string | null> {
    const dir = this.monitor.running ? this.monitor.dir : null;
    this.monitor = { running: false, dir: this.monitor.dir };
    return dir;
  }

  async monitorStatus(): Promise<MonitorStatus> {
    return { ...this.monitor };
  }

  private async startEventLog(args: EventLogStartArgs): Promise<EventLogStarted> {
    const started = await this.eventLog.start(args);
    this.calls.push(`eventLogStart:${started.dir}`);
    return started;
  }

  // The Event Log follower goes through `toolInvoke`, like the real backend, so availability applies.
  eventLogChannels(): Promise<EventLogChannel[]> {
    return this.toolInvoke(TOOL_IDS.eventLog, 'channels');
  }
  eventLogValidate(query: EventLogQuery): Promise<EventLogValidation> {
    return this.toolInvoke(TOOL_IDS.eventLog, 'validate', query);
  }
  eventLogStart(args: EventLogStartArgs): Promise<EventLogStarted> {
    return this.toolInvoke(TOOL_IDS.eventLog, 'start', args);
  }
  eventLogStop(): Promise<string | null> {
    return this.toolInvoke(TOOL_IDS.eventLog, 'stop');
  }
  eventLogPause(): Promise<EventLogStatus> {
    return this.toolInvoke(TOOL_IDS.eventLog, 'pause');
  }
  eventLogResume(): Promise<EventLogStatus> {
    return this.toolInvoke(TOOL_IDS.eventLog, 'resume');
  }
  eventLogStatus(): Promise<EventLogStatus> {
    return this.toolInvoke(TOOL_IDS.eventLog, 'status');
  }

  async collectDefaultPlan(): Promise<string> {
    return JSON.stringify(DEFAULT_PLAN);
  }

  async collectDiagnostics(
    plan: string,
    outDir: string,
    overwrite?: boolean,
  ): Promise<CollectResult> {
    let parsed: CollectPlan;
    try {
      parsed = JSON.parse(plan) as CollectPlan;
    } catch (e) {
      throw new Error(`the plan is not valid JSON: ${e instanceof Error ? e.message : String(e)}`, { cause: e });
    }
    if (!outDir.trim()) throw new Error('choose an output folder');
    // The mock's bundle name has no timestamp, so a second run into the same folder meets the first bundle.
    const bundle = `${outDir}/${parsed.name}.imtz`;
    this.files.write(bundle, overwrite);
    return this.slow({ bundle, manifest: mockManifest(parsed) });
  }

  async elevation(): Promise<ElevationInfo> {
    const windows = this.options.platform === 'windows';
    return {
      elevated: this.options.elevated,
      canRestartElevated: windows,
      platform: this.options.platform,
    };
  }

  async restartElevated(reopenSession: boolean): Promise<void> {
    if (this.options.platform !== 'windows')
      throw new Error('restarting as administrator is only available on Windows');
    this.calls.push(`restartElevated:${reopenSession ? 'session' : ''}`);
  }

  async appStatus(): Promise<AppStatus> {
    return {
      version: '0.1.0',
      safeMode: this.options.safeMode,
      previousCrash: this.options.previousCrash,
      sessionAvailable: this.session !== null || this.options.previousCrash,
    };
  }

  async restartApp(safeMode: boolean, reopenSession: boolean): Promise<void> {
    this.calls.push(`restartApp:${safeMode}:${reopenSession ? 'session' : ''}`);
  }

  async pluginReset(id: string): Promise<void> {
    this.calls.push(`pluginReset:${id}`);
  }

  async pluginSetEnabled(id: string, enabled: boolean): Promise<boolean> {
    if (enabled) this.disabled.delete(id);
    else this.disabled.add(id);
    this.calls.push(`pluginSetEnabled:${id}:${enabled}`);
    return true;
  }

  async saveSession(ui: unknown, reports?: Report[]): Promise<void> {
    this.session = ui;
    this.sessionReports = structuredClone(reports ?? []);
    this.sessionClock = await this.clockOf();
    this.calls.push('saveSession');
  }

  async lastSession(): Promise<string | null> {
    return this.session !== null || this.options.previousCrash
      ? '/demo/session/last-session.imtproj'
      : null;
  }

  async saveProject(
    path: string,
    name: string,
    ui: unknown,
    reports?: Report[],
    overwrite = false,
  ): Promise<ProjectSaved> {
    // Like the engine: a project is one self-contained `.imtz` file.
    if (!path.toLowerCase().endsWith('.imtz')) throw new Error('a project must be saved as an .imtz file');
    this.files.write(path, overwrite);
    this.projects.set(path, {
      name,
      ui: structuredClone(ui),
      reports: structuredClone(reports ?? []),
      clock: await this.clockOf(),
    });
    this.calls.push(`saveProject:${path}`);
    return { path, name };
  }

  async openProject(path: string): Promise<ProjectOpened> {
    const found =
      this.projects.get(path) ??
      (path.endsWith('last-session.imtproj')
        ? {
            name: 'Last session',
            ui: this.session,
            reports: this.sessionReports,
            clock: this.sessionClock,
          }
        : null);
    if (!found) throw new Error(`${path} is not an IMT project`);
    return {
      path,
      name: found.name,
      summary: await this.restoreClock(found.clock ?? {}),
      profile: { name: 'default', path: null, toml: '' },
      ui: structuredClone(found.ui),
      missing: path.includes('missing') ? ['/demo/gone/app.log'] : [],
      reports: structuredClone(found.reports ?? []),
      readOnly: false,
    };
  }

  async takePendingOpen(): Promise<string[]> {
    return [];
  }

  async onOpenRequested(): Promise<Unlisten> {
    return () => undefined;
  }
}
