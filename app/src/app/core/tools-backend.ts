import { invoke } from './activity/ipc';
import { inject, Provider } from '@angular/core';
import { Report } from './api.types';
import { ImtBackend } from './backend';
import { reportBackendFailures } from './failure-reporter';
import { Logger, withFailureLogging } from './logger';
import { isTauri } from './platform';
import { MockToolsBackend } from '../tools/mock/mock-tools-backend';
import { listenHere } from '../windowing/window-events';
import {
  AppStatus,
  CollectorAction,
  CollectorStatus,
  CollectorTemplate,
  CollectResult,
  CounterPreset,
  ElevationInfo,
  GeneratedCollector,
  MonitorConfig,
  MonitorStatus,
  ProjectOpened,
  ProjectSaved,
  ToolEntry,
} from './tools.types';
import { hasSkew } from './skew-backend';

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

/** App lifetime: elevation, safe mode, plugin settings and session recovery. */
export abstract class LifecycleBackend {
  abstract elevation(): Promise<ElevationInfo>;
  abstract restartElevated(reopen: string[]): Promise<void>;
  abstract appStatus(): Promise<AppStatus>;
  abstract restartApp(safeMode: boolean, reopen: string[]): Promise<void>;
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
  implements ToolCatalogBackend, PerfmonBackend, CollectBackend, LifecycleBackend, ProjectBackend
{
  abstract toolsList(): Promise<ToolEntry[]>;
  abstract toolInvoke<T = unknown>(tool: string, op: string, args?: unknown): Promise<T>;
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
  abstract elevation(): Promise<ElevationInfo>;
  abstract restartElevated(reopen: string[]): Promise<void>;
  abstract appStatus(): Promise<AppStatus>;
  abstract restartApp(safeMode: boolean, reopen: string[]): Promise<void>;
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
  // The specific commands below are deprecated wrappers over `tool_invoke` in the shell; kept so
  // the UI and its tests work unchanged.
  perfmonPresets() {
    return this.call<CounterPreset[]>('perfmon_presets');
  }
  perfmonGenerate(template: CollectorTemplate, xmlPath?: string, overwrite = false) {
    return this.call<GeneratedCollector>('perfmon_generate', {
      template,
      xmlPath,
      ...(overwrite ? { overwrite } : {}),
    });
  }
  perfmonInstall(template: CollectorTemplate) {
    return this.call<CollectorStatus>('perfmon_install', { template });
  }
  perfmonControl(name: string, action: CollectorAction) {
    return this.call<CollectorStatus | null>('perfmon_control', { name, action });
  }
  perfmonOutputs(name: string) {
    return this.call<string[]>('perfmon_outputs', { name });
  }
  monitorStart(config: MonitorConfig, dir?: string) {
    return this.call<string>('monitor_start', { config, dir });
  }
  monitorStop() {
    return this.call<string | null>('monitor_stop');
  }
  monitorStatus() {
    return this.call<MonitorStatus>('monitor_status');
  }
  collectDefaultPlan() {
    return this.call<string>('collect_default_plan');
  }
  collectDiagnostics(plan: string, outDir: string, overwrite = false) {
    return this.call<CollectResult>('collect_diagnostics', {
      plan,
      outDir,
      ...(overwrite ? { overwrite } : {}),
    });
  }
  elevation() {
    return this.call<ElevationInfo>('elevation');
  }
  restartElevated(reopen: string[]) {
    return this.call<void>('restart_elevated', { reopen });
  }
  appStatus() {
    return this.call<AppStatus>('app_status');
  }
  restartApp(safeMode: boolean, reopen: string[]) {
    return this.call<void>('restart_app', { safeMode, reopen });
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
      : new MockToolsBackend(
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
    { provide: LifecycleBackend, useExisting: ToolsBackend },
    { provide: ProjectBackend, useExisting: ToolsBackend },
  ];
}
