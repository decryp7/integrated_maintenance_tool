import { inject, Provider } from '@angular/core';
import { ImtBackend } from '../../core/backend';
import { MockAnalysisBackend, recordsFrom } from '../../analysis/mock/mock-analysis-backend';
import { MockShareBackend } from '../mock/mock-share-backend';
import { ShareBackend } from '../share-backend';

/** The mock share backend over the test's ImtBackend (unit tests of share features and their hosts). */
export function provideMockShare(): Provider[] {
  return [{ provide: ShareBackend, useFactory: () => new MockShareBackend(inject(ImtBackend), new MockAnalysisBackend(recordsFrom(inject(ImtBackend)))) }];
}
