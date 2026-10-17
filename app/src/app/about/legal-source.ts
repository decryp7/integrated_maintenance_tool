import { InjectionToken } from '@angular/core';

/** The licence documents the About dialog can show. */
export type LegalDoc = 'notices' | 'mit' | 'apache';

/** Where the About dialog gets its legal texts from (swapped for a fake in tests). */
export interface LegalSource {
  load(doc: LegalDoc): Promise<string>;
}

/**
 * Texts compiled into lazy chunks by `scripts/notices/generate.sh`: they load without fetch (which the
 * production CSP's `connect-src` forbids) and cost nothing until About is opened.
 */
export const bundledLegalSource: LegalSource = {
  load(doc) {
    switch (doc) {
      case 'notices':
        return import('./generated/notices.generated').then((m) => m.THIRD_PARTY_NOTICES);
      case 'mit':
        return import('./generated/licenses.generated').then((m) => m.LICENSE_MIT);
      case 'apache':
        return import('./generated/licenses.generated').then((m) => m.LICENSE_APACHE);
    }
  },
};

export const LEGAL_SOURCE = new InjectionToken<LegalSource>('LEGAL_SOURCE', { providedIn: 'root', factory: () => bundledLegalSource });
