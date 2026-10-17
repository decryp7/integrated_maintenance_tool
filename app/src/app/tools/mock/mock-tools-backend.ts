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
  GeneratedCollector,
  MonitorConfig,
  MonitorStatus,
  Platform,
  ProjectOpened,
  ProjectSaved,
} from '../../core/tools.types';
import type { ToolsBackend, Unlisten } from '../../core/tools-backend';
import { DEFAULT_PLAN } from '../collector/plan-model';

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
}

export const MOCK_DEFAULTS: MockToolsOptions = { safeMode: false, previousCrash: false, elevated: false, platform: 'windows', denied: false, latency: 0 };

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
  };
}

const PRESETS: CounterPreset[] = [
  {
    id: 'cpu',
    name: 'CPU',
    description: 'Per-process and machine CPU use, run queue and context switches.',
    counters: [
      { path: '\\Process({process})\\% Processor Time', guidance: 'Sustained near 100% per core means the process is CPU bound.' },
      { path: '\\Process({process})\\Thread Count', guidance: 'A steady climb signals a thread leak.' },
      { path: '\\System\\Processor Queue Length', guidance: 'More than 2 per core for long stretches means threads wait for a CPU.' },
    ],
  },
  {
    id: 'memory',
    name: 'Memory',
    description: 'Per-process memory, handles and pools; finds leaks.',
    counters: [
      { path: '\\Process({process})\\Private Bytes', guidance: 'A line that only rises, never returning after load, is a memory leak.' },
      { path: '\\Process({process})\\Handle Count', guidance: 'Continuous growth means leaked handles.' },
      { path: '\\Memory\\Available MBytes', guidance: null },
    ],
  },
  {
    id: 'disk',
    name: 'Disk',
    description: 'Per-process I/O volume and physical disk latency.',
    counters: [{ path: '\\PhysicalDisk(_Total)\\Avg. Disk sec/Read', guidance: 'Above 20 ms sustained is a slow disk.' }],
  },
];

const FORMAT_CODE = { csv: '0', tsv: '1', blg: '3' } as const;

function escapeXml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

/** A plausible Data Collector Set document (the real one is produced by the Rust engine). */
export function mockXml(t: CollectorTemplate): string {
  const all = PRESETS.filter((p) => t.presets.includes(p.id)).flatMap((p) => p.counters.map((c) => c.path));
  const paths = [...all, ...t.extraCounters];
  const targets = t.processes.length ? t.processes : ['*'];
  const counters = targets.flatMap((proc) => paths.map((p) => `    <Counter>${escapeXml(p.replace('{process}', proc))}</Counter>`));
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
      { path: 'logs/app.log', kind: 'log', origin: plan.logs[0]?.path ?? 'C:\\logs', originalBytes: 52_000, bytes: 51_200, redacted: true, redactions: 3 },
      { path: 'environment.json', kind: 'environment', origin: 'environment', originalBytes: 2_100, bytes: 2_100, redacted: false, redactions: 0 },
    ],
    skipped: plan.msinfo ? [] : [{ kind: 'msinfo', what: 'msinfo32', reason: 'disabled in the plan' }],
    redaction: { email: 2, token: 1 },
    errors: [],
    totalBytes: 53_300,
    durationMs: 420,
  };
}

/** In-memory stand-in for the tools commands: plausible behaviour for browser development and UI tests. */
export class MockToolsBackend implements ToolsBackend {
  private readonly sets = new Map<string, CollectorStatus>();
  private monitor: MonitorStatus = { running: false, dir: null };
  private session: unknown = null;
  private readonly projects = new Map<string, { name: string; ui: unknown }>();
  private readonly disabled = new Set<string>();
  private monitorCount = 0;
  /** Everything the UI asked to be done, for assertions. */
  readonly calls: string[] = [];

  constructor(
    readonly options: MockToolsOptions = optionsFromSearch(typeof location === 'undefined' ? '' : location.search),
    private readonly summaryOf: () => Promise<ProjectOpened['summary']> = () => Promise.reject(new Error('no workspace summary available')),
  ) {}

  private async slow<T>(value: T): Promise<T> {
    if (this.options.latency) await new Promise((r) => setTimeout(r, this.options.latency));
    return value;
  }

  private requireAdmin(): void {
    if (this.options.denied && !this.options.elevated) throw new Error('Access is denied. Administrator rights are required.');
  }

  private requireWindows(): void {
    if (this.options.platform !== 'windows') throw new Error('Performance Monitor collector sets can only be managed on Windows');
  }

  async perfmonPresets(): Promise<CounterPreset[]> {
    return structuredClone(PRESETS);
  }

  async perfmonGenerate(template: CollectorTemplate, xmlPath?: string): Promise<GeneratedCollector> {
    this.validate(template);
    const path = xmlPath ?? `${template.name}.xml`;
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
    const status: CollectorStatus = { name: template.name, status: 'stopped', rootPath: template.outputDir, outputLocation: `${template.outputDir}${template.name}`, segment: false };
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
    if (t.sampleIntervalSecs < 1) throw new Error('the sample interval must be between 1 and 86400 seconds');
    if (t.maxSizeMb != null && t.format !== 'blg') throw new Error('a circular log (maximum size) needs the binary BLG format');
  }

  async monitorStart(config: MonitorConfig, dir?: string): Promise<string> {
    if (this.monitor.running) throw new Error('the process monitor is already running');
    if (!config.targets.length) throw new Error('invalid monitor config: add at least one target');
    const folder = dir ?? `/demo/monitor/run-${++this.monitorCount}`;
    this.monitor = { running: true, dir: folder };
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

  async collectDefaultPlan(): Promise<string> {
    return JSON.stringify(DEFAULT_PLAN);
  }

  async collectDiagnostics(plan: string, outDir: string): Promise<CollectResult> {
    let parsed: CollectPlan;
    try {
      parsed = JSON.parse(plan) as CollectPlan;
    } catch (e) {
      throw new Error(`the plan is not valid JSON: ${e instanceof Error ? e.message : String(e)}`);
    }
    if (!outDir.trim()) throw new Error('choose an output folder');
    return this.slow({ bundle: `${outDir}/${parsed.name}.imtz`, manifest: mockManifest(parsed) });
  }

  async elevation(): Promise<ElevationInfo> {
    const windows = this.options.platform === 'windows';
    return { elevated: this.options.elevated, canRestartElevated: windows, platform: this.options.platform };
  }

  async restartElevated(reopen: string[]): Promise<void> {
    if (this.options.platform !== 'windows') throw new Error('restarting as administrator is only available on Windows');
    this.calls.push(`restartElevated:${reopen.join('|')}`);
  }

  async appStatus(): Promise<AppStatus> {
    return { version: '0.1.0', safeMode: this.options.safeMode, previousCrash: this.options.previousCrash, sessionAvailable: this.session !== null || this.options.previousCrash };
  }

  async restartApp(safeMode: boolean, reopen: string[]): Promise<void> {
    this.calls.push(`restartApp:${safeMode}:${reopen.join('|')}`);
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

  async saveSession(ui: unknown): Promise<void> {
    this.session = ui;
    this.calls.push('saveSession');
  }

  async lastSession(): Promise<string | null> {
    return this.session !== null || this.options.previousCrash ? '/demo/session/last-session.imtproj' : null;
  }

  async saveProject(path: string, name: string, ui: unknown): Promise<ProjectSaved> {
    this.projects.set(path, { name, ui: structuredClone(ui) });
    this.calls.push(`saveProject:${path}`);
    return { path, name };
  }

  async openProject(path: string): Promise<ProjectOpened> {
    const found = this.projects.get(path) ?? (path.endsWith('last-session.imtproj') ? { name: 'Last session', ui: this.session } : null);
    if (!found) throw new Error(`${path} is not an IMT project`);
    return {
      path,
      name: found.name,
      summary: await this.summaryOf(),
      profile: { name: 'default', path: null, toml: '' },
      ui: structuredClone(found.ui),
      missing: path.includes('missing') ? ['/demo/gone/app.log'] : [],
    };
  }

  async setLive(enabled: boolean): Promise<number> {
    this.calls.push(`setLive:${enabled}`);
    return enabled ? 1 : 0;
  }

  async takePendingOpen(): Promise<string[]> {
    return [];
  }

  async onOpenRequested(): Promise<Unlisten> {
    return () => undefined;
  }

  async onWorkspaceChanged(): Promise<Unlisten> {
    return () => undefined;
  }
}
