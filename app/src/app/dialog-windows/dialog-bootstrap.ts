import { ApplicationConfig, EnvironmentProviders, inject, Injector, provideAppInitializer, provideBrowserGlobalErrorListeners, Provider } from '@angular/core';
import { provideActivity } from '../core/activity/provide-activity';
import { provideBackend } from '../core/backend';
import { provideErrorLogging, provideLogger } from '../core/logger';
import { provideNativeFeel } from '../core/native-feel';
import { WORKSPACE_QUERIES, WORKSPACE_SUMMARY } from '../core/workspace/workspace-queries';
import { ThemeService } from '../core/workspace/theme.service';
import { UiSync } from '../popout/ui-sync';
import { provideDebriefIcons } from '../shared/material/debrief-icons';
import { DialogBackend } from './dialog-def';
import { HOST_COMMAND_FORWARD } from './host-commands';
import { dialogDef } from './dialog-registry';

type Providers = (Provider | EnvironmentProviders)[];

/** Backend provider groups, each loaded only by the dialogs that name it in `backends`. */
const BACKENDS: Record<DialogBackend, () => Promise<Providers>> = {
  ai: () => import('../ai/provide-ai').then((m) => [m.provideAi()]),
  tools: () => import('../core/tools-backend').then((m) => m.provideToolsBackend()),
  share: async () => {
    const { ShareBackend } = await import('../share/share-backend');
    const { createShareBackend } = await import('../share/share.plugin');
    return [{ provide: ShareBackend, useFactory: createShareBackend }];
  },
  updates: async () => {
    const { UpdatesBackend } = await import('../updates/updates-backend');
    const { createUpdatesBackend } = await import('../updates/updates-backend');
    return [{ provide: UpdatesBackend, useFactory: createUpdatesBackend }];
  },
  diagnostics: async () => {
    const { SnapshotBackend } = await import('../diagnostics/snapshot-backend');
    const { createSnapshotBackend } = await import('../diagnostics/provide-diagnostics');
    return [{ provide: SnapshotBackend, useFactory: createSnapshotBackend }];
  },
  analysis: async () => {
    const { AnalysisBackend } = await import('../core/analysis-backend');
    const { createAnalysisBackend } = await import('../analysis/analysis.plugin');
    return [{ provide: AnalysisBackend, useFactory: createAnalysisBackend }];
  },
  // The dialog shows the workspace's own data (timeline, ...): the window fetches it like the main window does.
  workspace: async () => [{ provide: WORKSPACE_QUERIES, useValue: true }, { provide: WORKSPACE_SUMMARY, useValue: true }],
  // The dialog reads the workspace's summary (sources, fields) but not its timeline.
  summary: async () => [{ provide: WORKSPACE_SUMMARY, useValue: true }],
  skew: () => import('../core/skew-backend').then((m) => [m.provideSkewBackend()]),
  profiling: () => import('../profiling/profiling-backend').then((m) => m.provideProfilingBackend()),
  native: () => import('../profiling/native/native-backend').then((m) => m.provideNativeBackend()),
  dotnet: () => import('../profiling/dotnet/dotnet-backend').then((m) => m.provideDotnetBackend()),
};

/**
 * The providers of a dialog window (ADR 0050, rule: each OS window is its own small bootstrap). The engine is the one
 * source of truth: a dialog window has the logger, the backends (commands to the engine), icons and the theme, and
 * shares UI state with its host through events. It does not have the main window's session, docking, windowing,
 * update, job or notification machinery, nor any other window's stores. Features beyond that are named by the dialog in
 * the registry and loaded by id.
 */
export async function dialogAppConfig(dialogId: string): Promise<ApplicationConfig> {
  const features = await Promise.all((dialogDef(dialogId)?.backends ?? []).map((b) => BACKENDS[b]()));
  return { providers: [...dialogBaseProviders(), ...features.flat()] };
}

export function dialogBaseProviders(): Providers {
  return [
    provideBrowserGlobalErrorListeners(),
    provideNativeFeel(),
    provideLogger(),
    provideErrorLogging(),
    { provide: WORKSPACE_QUERIES, useValue: false },
    { provide: WORKSPACE_SUMMARY, useValue: false },
    provideBackend(),
    provideActivity(),
    provideDebriefIcons(),
    provideAppInitializer(() => inject(ThemeService).start()),
    // Commands act on the window that opened the dialog.
    { provide: HOST_COMMAND_FORWARD, useFactory: () => { const injector = inject(Injector); return (id: string) => injector.get(UiSync).runInHost('dialogRun', id); } },
  ];
}
