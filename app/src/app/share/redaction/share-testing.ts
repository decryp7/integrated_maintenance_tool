import { inject, Provider } from '@angular/core';
import { DebriefBackend } from '../../core/backend';
import { MockAnalysisBackend, recordsFrom } from '../../analysis/mock/mock-analysis-backend';
import { MockShareBackend } from '../mock/mock-share-backend';
import { ShareBackend } from '../share-backend';

/** The mock share backend over the test's DebriefBackend (unit tests of share features and their hosts). */
export function provideMockShare(): Provider[] {
  return [{ provide: ShareBackend, useFactory: () => new MockShareBackend(inject(DebriefBackend), new MockAnalysisBackend(recordsFrom(inject(DebriefBackend)))) }];
}
