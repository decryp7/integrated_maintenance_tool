import { EnvironmentProviders, makeEnvironmentProviders } from '@angular/core';

/** One line in `app.config.ts`: the playbooks feature (ADR 0039). Filled in by its owner. */
export function providePlaybooks(): EnvironmentProviders {
  return makeEnvironmentProviders([]);
}
