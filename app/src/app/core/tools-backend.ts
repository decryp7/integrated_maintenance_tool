import { invoke } from './activity/ipc';
import { inject, Provider } from '@angular/core';
import { Report } from './api.types';
import { ImtBackend } from './backend';
import { reportBackendFailures } from './failure-reporter';
import { Logger, withFailureLogging } from './logger';
import { isTauri } from './platform';
import { listenHere } from '../windowing/window-events';
import {
  AppStatus,
  CollectorAction,
  CollectorStatus,
  CollectorTemplate,
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
  ProjectOpened,
  ProjectSaved,
  TOOL_IDS,
  ToolEntry,
} from './tools.types';
import { hasSkew } from './skew-backend';
import { browserMocks } from './mock-registry';

/** Names of the events the shell emits. */
export const OPEN_EVENT = 'imt://open';

/** Stops listening to an event. */
export type Unlisten = () => void;

/** Plugin tools: what is installed and usable, and the generic call. */
export abstract class ToolCatalogBackend {
  /** The tools of this window's plugins with their availability (empty in safe mode). */
  abstract toolsList(): Promise<ToolEntry[]>;
  /** Runs one operation of a tool (`tool_invoke`); `args` must match the op's schema. */
  abstract toolInvoke<T = unknown>(tool: string, op: string, args?: unknown): Promise<T>;
  /** Runs a tool's pollable status operation (`tool_poll`, quiet in the activity log). */
  abstract toolPoll<T = unknown>(tool: string, op: string, args?: unknown): Promise<T>;
  /** The job of a running tool task (cancel it with `jobs_cancel`), or null. */
  abstract toolTaskJob(task: string): Promise<number | null>;
}

/** Performance Monitor templates and `logman` control. */
export abstract class PerfmonBackend {
  abstract perfmonPresets(): Promise<CounterPreset[]>;
  /** Builds the XML; with `xmlPath` also writes it there, replacing an existing file only with `overwrite`. */
  abstract perfmonGenerate(
    template: CollectorTemplate,
    xmlPath?: string,
    overwrite?: boolean,
  ): Promise<GeneratedCollector>;
  abstract perfmonInstall(template: CollectorTemplate): Promise<CollectorStatus>;
  abstract perfmonControl(name: string, action: CollectorAction): Promise<CollectorStatus | null>;
  abstract perfmonOutputs(name: string): Promise<string[]>;
}

/** Process monitor and diagnostics collector. */
export abstract class CollectBackend {
  abstract monitorStart(config: MonitorConfig, dir?: string): Promise<string>;
  abstract monitorStop(): Promise<string | null>;
  abstract monitorStatus(): Promise<MonitorStatus>;
  abstract collectDefaultPlan(): Promise<string>;
  /** Writes a bundle into `outDir`; an existing bundle of the same name is replaced only with `overwrite`. */
  abstract collectDiagnostics(
    plan: string,
    outDir: string,
    overwrite?: boolean,
  ): Promise<CollectResult>;
}

/** The Windows Event Log follower (tool `event-log-follower`, reached through `tool_invoke`). */
export abstract class EventLogBackend {
  /** The channels worth following, standard ones first. */
  abstract eventLogChannels(): Promise<EventLogChannel[]>;
  /** Composes the queries and checks each channel against them, without starting anything. */
  abstract eventLogValidate(query: EventLogQuery): Promise<EventLogValidation>;
  abstract eventLogStart(args: EventLogStartArgs): Promise<EventLogStarted>;
  /** Stops the run; its folder, or null when nothing ran. */
  abstract eventLogStop(): Promise<string | null>;
  abstract eventLogPause(): Promise<EventLogStatus>;
  abstract eventLogResume(): Promise<EventLogStatus>;
  abstract eventLogStatus(): Promise<EventLogStatus>;
}

/** App lifetime: elevation, safe mode, plugin settings and session recovery. */
export abstract class LifecycleBackend {
  abstract elevation(): Promise<ElevationInfo>;
  /** Relaunches elevated; `reopenSession` reopens the autosaved session (Rust builds the command line). */
  abstract restartElevated(reopenSession: boolean): Promise<void>;
  abstract appStatus(): Promise<AppStatus>;
  abstract restartApp(safeMode: boolean, reopenSession: boolean): Promise<void>;
  abstract pluginReset(id: string): Promise<void>;
  abstract pluginSetEnabled(id: string, enabled: boolean): Promise<boolean>;
  /** Autosaves the session; `reports` are the project's incident reports. */
  abstract saveSession(ui: unknown, reports?: Report[]): Promise<void>;
  abstract lastSession(): Promise<string | null>;
}

/** Projects and OS "open with" requests (live tail lives on `ImtBackend`). */
export abstract class ProjectBackend {
  /** Saves the project; `reports` are its incident reports (validated by the engine). Never replaces a file unless `overwrite`. */
  abstract saveProject(
    path: string,
    name: string,
    ui: unknown,
    reports?: Report[],
    overwrite?: boolean,
  ): Promise<ProjectSaved>;
  abstract openProject(path: string): Promise<ProjectOpened>;
  abstract takePendingOpen(): Promise<string[]>;
  /** Calls `handler` whenever the shell queued paths to open (`imt://open`). */
  abstract onOpenRequested(handler: () => void): Promise<Unlisten>;
}

/**
 * Everything the tools, lifecycle and project features ask of the backend. Consumers inject the
 * narrow part they use (`PerfmonBackend`, ...); `ToolsBackend` is the full class registered for DI.
 */
export abstract class ToolsBackend
  implements ToolCatalogBackend, PerfmonBackend, CollectBackend, EventLogBackend, LifecycleBackend, ProjectBackend
{
  abstract toolsList(): Promise<ToolEntry[]>;
  abstract toolInvoke<T = unknown>(tool: string, op: string, args?: unknown): Promise<T>;
  abstract toolPoll<T = unknown>(tool: string, op: string, args?: unknown): Promise<T>;
  abstract toolTaskJob(task: string): Promise<number | null>;
  abstract perfmonPresets(): Promise<CounterPreset[]>;
  /** Builds the XML; with `xmlPath` also writes it there, replacing an existing file only with `overwrite`. */
  abstract perfmonGenerate(
    template: CollectorTemplate,
    xmlPath?: string,
    overwrite?: boolean,
  ): Promise<GeneratedCollector>;
  abstract perfmonInstall(template: CollectorTemplate): Promise<CollectorStatus>;
  abstract perfmonControl(name: string, action: CollectorAction): Promise<CollectorStatus | null>;
  abstract perfmonOutputs(name: string): Promise<string[]>;
  abstract monitorStart(config: MonitorConfig, dir?: string): Promise<string>;
  abstract monitorStop(): Promise<string | null>;
  abstract monitorStatus(): Promise<MonitorStatus>;
  abstract collectDefaultPlan(): Promise<string>;
  /** Writes a bundle into `outDir`; an existing bundle of the same name is replaced only with `overwrite`. */
  abstract collectDiagnostics(
    plan: string,
    outDir: string,
    overwrite?: boolean,
  ): Promise<CollectResult>;
  abstract eventLogChannels(): Promise<EventLogChannel[]>;
  abstract eventLogValidate(query: EventLogQuery): Promise<EventLogValidation>;
  abstract eventLogStart(args: EventLogStartArgs): Promise<EventLogStarted>;
  abstract eventLogStop(): Promise<string | null>;
  abstract eventLogPause(): Promise<EventLogStatus>;
  abstract eventLogResume(): Promise<EventLogStatus>;
  abstract eventLogStatus(): Promise<EventLogStatus>;
  abstract elevation(): Promise<ElevationInfo>;
  /** Relaunches elevated; `reopenSession` reopens the autosaved session (Rust builds the command line). */
  abstract restartElevated(reopenSession: boolean): Promise<void>;
  abstract appStatus(): Promise<AppStatus>;
  abstract restartApp(safeMode: boolean, reopenSession: boolean): Promise<void>;
  abstract pluginReset(id: string): Promise<void>;
  abstract pluginSetEnabled(id: string, enabled: boolean): Promise<boolean>;
  /** Autosaves the session; `reports` are the project's incident reports. */
  abstract saveSession(ui: unknown, reports?: Report[]): Promise<void>;
  abstract lastSession(): Promise<string | null>;
  /** Saves the project; `reports` are its incident reports (validated by the engine). Never replaces a file unless `overwrite`. */
  abstract saveProject(
    path: string,
    name: string,
    ui: unknown,
    reports?: Report[],
    overwrite?: boolean,
  ): Promise<ProjectSaved>;
  abstract openProject(path: string): Promise<ProjectOpened>;
  abstract takePendingOpen(): Promise<string[]>;
  abstract onOpenRequested(handler: () => void): Promise<Unlisten>;
}

/** Subscribes to a Tauri event; tests pass a fake. */
export type ListenFn = (event: string, handler: () => void) => Promise<Unlisten>;

/* v8 ignore start: thin wrapper over the Tauri event API, exercised only in the real app */
const tauriListen: ListenFn = (event, handler) => listenHere(event, () => handler());
/* v8 ignore stop */

/** Talks to the Rust shell through Tauri commands. */
export class TauriToolsBackend extends ToolsBackend {
  /** `call` is Tauri's `invoke`, `listen` its event subscription; tests pass fakes. */
  constructor(
    private readonly call: typeof invoke = invoke,
    private readonly listen: ListenFn = tauriListen,
  ) {
    super();
  }

  toolsList() {
    return this.call<ToolEntry[]>('tools_list');
  }
  toolInvoke<T = unknown>(tool: string, op: string, args: unknown = {}) {
    return this.call<T>('tool_invoke', { tool, op, args });
  }
  toolPoll<T = unknown>(tool: string, op: string, args: unknown = {}) {
    return this.call<T>('tool_poll', { tool, op, args });
  }
  toolTaskJob(task: string) {
    return this.call<number | null>('tool_task_job', { task });
  }
  perfmonPresets() {
    return this.toolInvoke<CounterPreset[]>(TOOL_IDS.perfmonTemplates, 'presets');
  }
  perfmonGenerate(template: CollectorTemplate, xmlPath?: string, overwrite = false) {
    return this.toolInvoke<GeneratedCollector>(TOOL_IDS.perfmonTemplates, 'generate', {
      template,
      ...(xmlPath === undefined ? {} : { xmlPath }),
      ...(overwrite ? { overwrite } : {}),
    });
  }
  perfmonInstall(template: CollectorTemplate) {
    return this.toolInvoke<CollectorStatus>(TOOL_IDS.perfmonSets, 'install', { template });
  }
  perfmonControl(name: string, action: CollectorAction) {
    return this.toolInvoke<CollectorStatus | null>(TOOL_IDS.perfmonSets, 'control', { name, action });
  }
  perfmonOutputs(name: string) {
    return this.toolInvoke<string[]>(TOOL_IDS.perfmonSets, 'outputs', { name });
  }
  monitorStart(config: MonitorConfig, dir?: string) {
    return this.toolInvoke<string>(TOOL_IDS.processMonitor, 'start', { config, ...(dir === undefined ? {} : { dir }) });
  }
  monitorStop() {
    return this.toolInvoke<string | null>(TOOL_IDS.processMonitor, 'stop');
  }
  monitorStatus() {
    return this.toolInvoke<MonitorStatus>(TOOL_IDS.processMonitor, 'status');
  }
  collectDefaultPlan() {
    return this.toolInvoke<string>(TOOL_IDS.diagnostics, 'default-plan');
  }
  collectDiagnostics(plan: string, outDir: string, overwrite = false) {
    return this.toolInvoke<CollectResult>(TOOL_IDS.diagnostics, 'collect', {
      plan,
      outDir,
      ...(overwrite ? { overwrite } : {}),
    });
  }
  // The Event Log follower has no dedicated command: every operation is `tool_invoke`.
  eventLogChannels() {
    return this.toolInvoke<EventLogChannel[]>(TOOL_IDS.eventLog, 'channels');
  }
  eventLogValidate(query: EventLogQuery) {
    return this.toolInvoke<EventLogValidation>(TOOL_IDS.eventLog, 'validate', query);
  }
  eventLogStart(args: EventLogStartArgs) {
    return this.toolInvoke<EventLogStarted>(TOOL_IDS.eventLog, 'start', args);
  }
  eventLogStop() {
    return this.toolInvoke<string | null>(TOOL_IDS.eventLog, 'stop');
  }
  eventLogPause() {
    return this.toolInvoke<EventLogStatus>(TOOL_IDS.eventLog, 'pause');
  }
  eventLogResume() {
    return this.toolInvoke<EventLogStatus>(TOOL_IDS.eventLog, 'resume');
  }
  eventLogStatus() {
    return this.toolInvoke<EventLogStatus>(TOOL_IDS.eventLog, 'status');
  }
  elevation() {
    return this.call<ElevationInfo>('elevation');
  }
  restartElevated(reopenSession: boolean) {
    return this.call<void>('restart_elevated', { reopenSession });
  }
  appStatus() {
    return this.call<AppStatus>('app_status');
  }
  restartApp(safeMode: boolean, reopenSession: boolean) {
    return this.call<void>('restart_app', { safeMode, reopenSession });
  }
  pluginReset(id: string) {
    return this.call<void>('plugin_reset', { id });
  }
  pluginSetEnabled(id: string, enabled: boolean) {
    return this.call<boolean>('plugin_set_enabled', { id, enabled });
  }
  saveSession(ui: unknown, reports?: Report[]) {
    return this.call<void>('save_session', reports ? { ui, reports } : { ui });
  }
  lastSession() {
    return this.call<string | null>('last_session');
  }
  saveProject(path: string, name: string, ui: unknown, reports?: Report[], overwrite = false) {
    return this.call<ProjectSaved>('save_project', {
      path,
      name,
      ui,
      ...(reports ? { reports } : {}),
      ...(overwrite ? { overwrite } : {}),
    });
  }
  openProject(path: string) {
    return this.call<ProjectOpened>('open_project', { path });
  }
  takePendingOpen() {
    return this.call<string[]>('take_pending_open');
  }
  onOpenRequested(handler: () => void) {
    return this.listen(OPEN_EVENT, handler);
  }
}

/** The real backend inside Tauri, the in-memory mock in a plain browser (ng serve, e2e). */
export function provideToolsBackend(): Provider[] {
  const factory = () => {
    const imt = inject(ImtBackend);
    const real = isTauri()
      ? new TauriToolsBackend()
      : new (browserMocks().MockToolsBackend)(
          undefined,
          () => imt.workspaceSummary(),
          hasSkew(imt)
            ? (source, offsetMs) => imt.setSourceOffset({ kind: 'manual', source, offsetMs })
            : undefined,
        );
    return withFailureLogging<ToolsBackend>(real, inject(Logger), {
      onFailure: reportBackendFailures(),
    });
  };
  return [
    { provide: ToolsBackend, useFactory: factory },
    { provide: ToolCatalogBackend, useExisting: ToolsBackend },
    { provide: PerfmonBackend, useExisting: ToolsBackend },
    { provide: CollectBackend, useExisting: ToolsBackend },
    { provide: EventLogBackend, useExisting: ToolsBackend },
    { provide: LifecycleBackend, useExisting: ToolsBackend },
    { provide: ProjectBackend, useExisting: ToolsBackend },
  ];
}
