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

const STAND_IN_NOTICES = [
  { kind: 'crate', name: 'serde', version: '1.0', licence: 'MIT OR Apache-2.0' },
  { kind: 'crate', name: 'serde_json', version: '1.0', licence: 'MIT OR Apache-2.0' },
  { kind: 'crate', name: 'tokio', version: '1.4', licence: 'MIT' },
  { kind: 'crate', name: 'regex', version: '1.1', licence: 'MIT OR Apache-2.0' },
  { kind: 'crate', name: 'memchr', version: '2.7', licence: 'Unlicense OR MIT' },
  { kind: 'crate', name: 'rayon', version: '1.1', licence: 'MIT OR Apache-2.0' },
  { kind: 'crate', name: 'duckdb', version: '1.1', licence: 'MIT' },
  { kind: 'crate', name: 'tauri', version: '2.0', licence: 'MIT OR Apache-2.0' },
  { kind: 'crate', name: 'postcard', version: '1.0', licence: 'MIT OR Apache-2.0' },
  { kind: 'crate', name: 'chrono', version: '0.4', licence: 'MIT OR Apache-2.0' },
  { kind: 'ui', name: 'Angular', version: '22', licence: 'MIT' },
  { kind: 'ui', name: 'Angular CDK', version: '22', licence: 'MIT' },
  { kind: 'ui', name: 'Apache ECharts', version: '6.1', licence: 'Apache-2.0' },
  { kind: 'ui', name: 'zrender', version: '6.1', licence: 'BSD-3-Clause' },
  { kind: 'ui', name: 'dockview', version: '4', licence: 'MIT' },
  { kind: 'ui', name: 'RxJS', version: '7.8', licence: 'Apache-2.0' },
] as const;

const line = (n: { name: string; version: string; licence: string }) => `- ${n.name} ${n.version} — ${n.licence}`;

/** The browser build (dev server, UI tests) has no engine: it shows a short stand-in. */
export const browserLegalSource: LegalSource = {
  async load(doc) {
    switch (doc) {
      case 'notices':
        return [
          '# Third-party notices',
          '',
          'The full list of third-party software, with every licence text, is shown by the desktop app. A sample of it:',
          '',
          '## Rust crates',
          '',
          ...STAND_IN_NOTICES.filter((n) => n.kind === 'crate').map(line),
          '',
          '## UI libraries',
          '',
          ...STAND_IN_NOTICES.filter((n) => n.kind === 'ui').map(line),
        ].join('\n');
      case 'mit':
        return 'MIT License\n\nThe full text is shown by the desktop app.';
      case 'apache':
        return 'Apache License\nVersion 2.0\n\nThe full text is shown by the desktop app.';
    }
  },
};

export const LEGAL_SOURCE = new InjectionToken<LegalSource>('LEGAL_SOURCE', { providedIn: 'root', factory: () => (isTauri() ? engineLegalSource : browserLegalSource) });
