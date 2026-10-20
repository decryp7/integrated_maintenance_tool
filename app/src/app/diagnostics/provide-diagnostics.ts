import { EnvironmentProviders, ErrorHandler, inject, Injector, makeEnvironmentProviders, provideAppInitializer } from '@angular/core';
import { ApplicationRef, createComponent, EnvironmentInjector } from '@angular/core';
import { FailureReporter } from '../core/failure-reporter';
import { Logger, LoggingErrorHandler, withFailureLogging } from '../core/logger';
import { isTauri } from '../core/platform';
import { IMT_COMMANDS, IMT_VIEWS } from '../extensions/tokens';
import { DiagnosticsCommands, DIAGNOSTICS_VIEWS } from './diagnostics-commands';
import { ErrorSnapshotService } from './error-snapshot.service';
import { SnapshotErrorHandler } from './snapshot-error-handler';
import { SnapshotBackend, TauriSnapshotBackend } from './snapshot-backend';
import { SnapshotToast } from './snapshot-toast';
import { DiagnosticsReportService } from './diagnostics-report.service';
import { ImtEvents } from '../extensions/events';
import { browserMocks } from '../core/mock-registry';

/** The real backend inside Tauri, an in-memory one in a plain browser (ng serve, e2e). */
export function createSnapshotBackend(): SnapshotBackend {
  const injector = inject(Injector);
  // The mock finds an opened diagnostics zip in the workspace; looked up late to avoid a cycle with the workspace store.
  const mock = () => new (browserMocks().MockSnapshotBackend)(undefined, () => injector.get(ImtEvents).workspace());
  return withFailureLogging<SnapshotBackend>(isTauri() ? new TauriSnapshotBackend() : mock(), inject(Logger), { category: 'ui.snapshot' });
}

/** Mounts the toast once over the app and starts listening. Not awaited: it must not delay the first paint. */
export function startDiagnostics(): void {
  const host = document.createElement('imt-snapshot-toast');
  document.body.append(host);
  const ref = createComponent(SnapshotToast, { environmentInjector: inject(EnvironmentInjector), hostElement: host });
  inject(ApplicationRef).attachView(ref.hostView);
  void inject(ErrorSnapshotService).start();
  inject(DiagnosticsReportService).start();
}

/**
 * Error snapshots: the view, the capture command (Ctrl/Cmd+Shift+F12), the toast, the `ErrorHandler` that
 * logs and then captures, the `imt://error` listener, and the `FailureReporter` that panels and backend
 * decorators report to. The snapshot backend itself is not reported (a failing save must not trigger a capture). Put it AFTER `provideErrorLogging()` in
 * `app.config.ts`, which it extends.
 */
export function provideDiagnostics(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: SnapshotBackend, useFactory: createSnapshotBackend },
    { provide: FailureReporter, useExisting: ErrorSnapshotService },
    LoggingErrorHandler,
    { provide: ErrorHandler, useClass: SnapshotErrorHandler },
    { provide: IMT_VIEWS, multi: true, useValue: DIAGNOSTICS_VIEWS },
    { provide: IMT_COMMANDS, multi: true, useFactory: () => inject(DiagnosticsCommands).commands },
    provideAppInitializer(() => startDiagnostics()),
  ]);
}
