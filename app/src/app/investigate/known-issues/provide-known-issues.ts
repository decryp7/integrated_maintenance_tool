import { EnvironmentProviders, makeEnvironmentProviders } from '@angular/core';

/** One line in `app.config.ts`: the known-issues feature (ADR 0039). Filled in by its owner. */
export function provideKnownIssues(): EnvironmentProviders {
  return makeEnvironmentProviders([]);
}
