import { inject, Injector } from '@angular/core';

/**
 * Receives failures the UI contained (a panel's error boundary, a rejected backend command), e.g. to take an
 * error snapshot. A port: core code depends on it, `provideDiagnostics()` binds it. It is optional: without a
 * provider, failures are only logged.
 */
export abstract class FailureReporter {
  abstract panelFailed(panel: string, error: unknown): void;
  abstract backendFailed(command: string, error: unknown): void;
}

/** Tells the reporter (when there is one) without ever throwing: reporting must not change the failure being reported. */
export function reportFailure(injector: Injector, report: (reporter: FailureReporter) => void): void {
  try {
    const reporter = injector.get(FailureReporter, null);
    if (reporter) report(reporter);
  } catch {
    /* a broken reporter must not break the caller */
  }
}

/**
 * An `onFailure` callback for `withFailureLogging` that forwards to the `FailureReporter`. The reporter is looked
 * up at call time, not when the backend is created, so a reporter that itself depends on backends causes no DI cycle.
 * Call in an injection context (a provider factory).
 */
export function reportBackendFailures(injector: Injector = inject(Injector)): (command: string, error: unknown) => void {
  return (command, error) => reportFailure(injector, (r) => r.backendFailed(command, error));
}
