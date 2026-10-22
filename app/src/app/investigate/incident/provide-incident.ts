import { EnvironmentProviders, makeEnvironmentProviders } from '@angular/core';

/** One line in `app.config.ts`: the incident feature (ADR 0039). Filled in by its owner. */
export function provideIncident(): EnvironmentProviders {
  return makeEnvironmentProviders([]);
}
