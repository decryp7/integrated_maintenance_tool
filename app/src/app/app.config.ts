import { ApplicationConfig, provideBrowserGlobalErrorListeners } from '@angular/core';
import { provideImtBuiltins } from './builtin/provide-builtins';
import { provideBackend } from './core/backend';
import { provideErrorLogging, provideLogger } from './core/logger';
import { provideImtPlugin } from './extensions';
import { provideDiagnostics } from './diagnostics/provide-diagnostics';
import { provideAnalysis } from './analysis/analysis.plugin';
import { provideLive } from './live/provide-live';
import { provideTools } from './tools/provide-tools';
import { provideSchedules } from './schedules/provide-schedules';
import { provideConfigDiff } from './config-diff/provide-config-diff';
import { provideMarkers } from './markers/markers.plugin';
import { provideRuns } from './timeline/runs.plugin';
import { REGISTRY_PLUGIN } from './plugins/registry/registry.plugin';
import { provideWindowing } from './windowing/provide-windowing';
import { provideClockSkew } from './skew/skew.plugin';
import { provideShare } from './share/share.plugin';
import { provideProfiling } from './profiling/provide-profiling';
import { provideDumps } from './dumps/provide-dumps';
import { provideHelp } from './help/help.commands';
import { provideUndo } from './undo/provide-undo';
import { provideOverview } from './overview/overview.plugin';
import { provideOnboarding } from './onboarding/onboarding.plugin';
import { provideDocking } from './dock/provide-docking';
import { provideTimelines } from './timeline/provide-timelines';
import { providePopout } from './popout/provide-popout';
import { provideAi } from './ai/provide-ai';
import { provideFolderSetup } from './ai/folder-setup/provide-folder-setup';
import { provideBrowser } from './browser/provide-browser';
import { provideActivity } from './core/activity/provide-activity';
import { provideImtIcons } from './shared/material/imt-icons';
import { provideUpdates } from './updates/provide-updates';
import { provideJobs } from './jobs/provide-jobs';
import { provideNotifications } from './notifications/provide-notifications';
import { provideNativeFeel } from './core/native-feel';
import { provideWorkspace } from './core/workspace/provide-workspace';
import { provideNotes } from './notes/provide-notes';
import { provideProblems } from './problems/provide-problems';
import { provideStorePane } from './store-pane/provide-store-pane';

export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    provideNativeFeel(),
    provideLogger(),
    provideErrorLogging(),
    provideDiagnostics(),
    provideBackend(),
    provideWorkspace(),
    provideImtBuiltins(),
    provideImtPlugin(REGISTRY_PLUGIN),
    provideMarkers(),
    provideRuns(),
    provideTools(),
    provideSchedules(),
    provideConfigDiff(),
    provideAnalysis(),
    provideLive(),
    provideWindowing(),
    provideClockSkew(),
    provideShare(),
    provideProfiling(),
    provideDumps(),
    provideHelp(),
    provideUndo(),
    provideOverview(),
    provideOnboarding(),
    provideDocking(),
    provideTimelines(),
    providePopout(),
    provideAi(),
    provideFolderSetup(),
    provideBrowser(),
    provideActivity(),
    provideImtIcons(),
    provideUpdates(),
    provideJobs(),
    provideNotifications(),
    provideNotes(),
    provideProblems(),
    provideStorePane(),
  ],
};
