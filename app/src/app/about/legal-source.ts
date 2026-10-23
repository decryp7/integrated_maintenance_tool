import { InjectionToken } from '@angular/core';
import { invoke } from '../core/activity/ipc';
import { isTauri } from '../core/platform';

/** The licence documents the About dialog can show. */
export type LegalDoc = 'notices' | 'mit' | 'apache';

/** Where the About dialog gets its legal texts from (swapped for a fake in tests). */
export interface LegalSource {
  load(doc: LegalDoc): Promise<string>;
}

/**
 * The desktop app asks the engine, which has the texts compiled in (`legal_text`): the ~15,000-line third-party notices
 * are never part of the UI's JavaScript and cost nothing until About is opened.
 */
export function engineLegalSourceWith(call: typeof invoke): LegalSource {
  return { load: (doc) => call<string>('legal_text', { doc }) };
}

export const engineLegalSource: LegalSource = engineLegalSourceWith(invoke);

/** The browser build (dev server, UI tests) has no engine: it shows a short stand-in. */
export const browserLegalSource: LegalSource = {
  async load(doc) {
    switch (doc) {
      case 'notices':
        return ['# Third-party notices', '', 'The full list of third-party software is shown by the desktop app.', '', '- serde_json 1.0.0 — MIT OR Apache-2.0', '- zrender 6.1.0 — BSD-3-Clause'].join('\n');
      case 'mit':
        return 'MIT License\n\nThe full text is shown by the desktop app.';
      case 'apache':
        return 'Apache License\nVersion 2.0\n\nThe full text is shown by the desktop app.';
    }
  },
};

export const LEGAL_SOURCE = new InjectionToken<LegalSource>('LEGAL_SOURCE', { providedIn: 'root', factory: () => (isTauri() ? engineLegalSource : browserLegalSource) });
