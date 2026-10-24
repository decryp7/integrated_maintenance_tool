import { ApplicationConfig, provideBrowserGlobalErrorListeners } from '@angular/core';
import { provideDebriefBuiltins } from './builtin/provide-builtins';
import { provideBackend } from './core/backend';
import { provideErrorLogging, provideLogger } from './core/logger';
import { provideDebriefPlugin } from './extensions';
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
import { provideWatched } from './watched/watched.plugin';
import { provideProfiling } from './profiling/provide-profiling';
import { provideDumps } from './dumps/provide-dumps';
import { provideVideo } from './video/provide-video';
import { provideHelp } from './help/help.commands';
import { provideNavigation } from './navigation/provide-navigation';
import { provideUndo } from './undo/provide-undo';
import { provideHome } from './home/home.plugin';
import { provideWhatsNew } from './whats-new/provide-whats-new';
import { provideOnboarding } from './onboarding/onboarding.plugin';
import { provideDocking } from './dock/provide-docking';
import { provideTimelines } from './timeline/provide-timelines';
import { providePopout } from './popout/provide-popout';
import { provideAi } from './ai/provide-ai';
import { provideFolderSetup } from './ai/folder-setup/provide-folder-setup';
import { provideBrowser } from './browser/provide-browser';
import { provideActivity } from './core/activity/provide-activity';
import { provideDebriefIcons } from './shared/material/debrief-icons';
import { provideUpdates } from './updates/provide-updates';
import { provideJobs } from './jobs/provide-jobs';
import { provideNotifications } from './notifications/provide-notifications';
import { provideNativeFeel } from './core/native-feel';
import { provideWorkspace } from './core/workspace/provide-workspace';
import { provideNotes } from './notes/provide-notes';
import { provideSearchEverything } from './search-everything/provide-search-everything';
import { provideProblems } from './problems/provide-problems';
import { provideIncident } from './investigate/incident/provide-incident';
import { provideKnownIssues } from './investigate/known-issues/provide-known-issues';
import { providePlaybooks } from './investigate/playbooks/provide-playbooks';
import { provideStorePane } from './store-pane/provide-store-pane';
import { provideNotificationHost, provideNotificationSource } from './health/provide-notification-host';
import { provideServiceHealth } from './core/service-health/provide-service-health';
import { WorkspaceSource } from './shell/workspace-source';
import { provideStartupContract } from './core/startup/startup-contract';
import { provideStartupTimings } from './core/startup/startup-timings';
import { provideRecovery } from './lifecycle/recovery';

export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    provideNativeFeel(),
    provideLogger(),
    provideErrorLogging(),
    provideDiagnostics(),
    provideBackend(),
    provideWorkspace(),
    provideDebriefBuiltins(),
    provideDebriefPlugin(REGISTRY_PLUGIN),
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
    provideWatched(),
    provideProfiling(),
    provideDumps(),
    provideVideo(),
    provideHelp(),
    provideUndo(),
    provideNavigation(),
    provideHome(),
    provideWhatsNew(),
    provideOnboarding(),
    provideDocking(),
    provideTimelines(),
    providePopout(),
    provideAi(),
    provideFolderSetup(),
    provideBrowser(),
    provideActivity(),
    provideDebriefIcons(),
    provideUpdates(),
    provideJobs(),
    provideNotifications(),
    provideNotes(),
    provideSearchEverything(),
    provideProblems(),
    provideKnownIssues(),
    provideIncident(),
    providePlaybooks(),
    provideStorePane(),
    provideServiceHealth(),
    provideNotificationHost(),
    provideNotificationSource(WorkspaceSource),
    // Reports ready after the first paint; Rust's watchdog recovers a window that never does (ADR 0058).
    provideStartupContract(),
    // The main window's start-up marks for Rust's "Startup timings" line (rule 16).
    provideStartupTimings(),
    provideRecovery(),
  ],
};
