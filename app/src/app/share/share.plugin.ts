import { EnvironmentProviders, inject, makeEnvironmentProviders } from '@angular/core';
import { AnalysisBackend } from '../core/analysis-backend';
import { DebriefBackend } from '../core/backend';
import { reportBackendFailures } from '../core/failure-reporter';
import { Logger, withFailureLogging } from '../core/logger';
import { isTauri } from '../core/platform';
import { DEBRIEF_RECORD_MENU_ITEMS } from '../context/record-actions';
import { SNAPSHOT_TEXT_FILTER } from '../diagnostics/error-snapshot.service';
import { DEBRIEF_COMMANDS } from '../extensions/tokens';
import { provideDebriefPlugin } from '../extensions/provide';
import { BundleCommands } from './bundle/bundle.commands';
import { DashboardCommands } from './dashboard/dashboard.commands';
import { RedactedCopyCommands } from './redacted-copy/redacted-copy-commands';
import { RedactionCommands } from './redaction/redaction.commands';
import { ReportCommands } from './report/report.commands';
import { reportMenuItems } from './report/report-menu';
import { ShareBackend, TauriShareBackend } from './share-backend';
import { SHARE_VIEWS } from './share.views';
import { snapshotRedaction } from './snapshot-redaction';
import { VisualCommands } from './visuals/visual.commands';
import { browserMocks } from '../core/mock-registry';

/** The engine-backed share backend inside Tauri, the mock (over the mock workspace) in a browser. */
export function createShareBackend(): ShareBackend {
  const backend: ShareBackend = isTauri() ? new TauriShareBackend() : new (browserMocks().MockShareBackend)(inject(DebriefBackend), inject(AnalysisBackend));
  return withFailureLogging<ShareBackend>(backend, inject(Logger), { onFailure: reportBackendFailures() });
}

/** One line in `app.config.ts`: redaction, dashboards, reports, images and bundles. */
export function provideShare(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: ShareBackend, useFactory: createShareBackend },
    { provide: SNAPSHOT_TEXT_FILTER, useFactory: snapshotRedaction },
    { provide: DEBRIEF_RECORD_MENU_ITEMS, multi: true, useFactory: reportMenuItems },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(RedactionCommands).commands },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(ReportCommands).commands },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(DashboardCommands).commands },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(BundleCommands).commands },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(VisualCommands).commands },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(RedactedCopyCommands).commands },
    provideDebriefPlugin({ views: SHARE_VIEWS }),
  ]);
}
