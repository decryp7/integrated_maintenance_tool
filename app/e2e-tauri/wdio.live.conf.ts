import { rmSync } from 'node:fs';
import { config as base } from './wdio.conf';
import { LIVE_DIR } from './support/paths';

// The live test of AI research against the real search engines (Google, Bing, DuckDuckGo, Stack Overflow) in the
// real app's page webview. Nightly and on manual runs only, never on push or pull requests: search engines
// rate-limit automated traffic and change their pages. See docs/development.md ("Live search test").
//
//   npm run e2e:tauri:build:live   # debug app binary with the `live-search-probe` feature
//   npm run e2e:tauri:live         # starts the app, waits for its report, judges every engine
//
// The app does the searching itself (`browser/live_probe.rs`, compiled only into this build): it reads
// IMT_E2E_LIVE_SEARCH_OUT at start-up and writes live-search.json and a screenshot per engine into it.

process.env['IMT_E2E_LIVE_SEARCH_OUT'] ??= LIVE_DIR;

export const config: WebdriverIO.Config = {
  ...base,
  specs: ['./live/**/*.e2e.ts'],
  // Four searches, each up to 15 s for the load and 15 s for the results, and a pause between them.
  mochaOpts: { ui: 'bdd', timeout: 300_000 },
  onPrepare(...args) {
    rmSync(LIVE_DIR, { recursive: true, force: true });
    const prepare = base.onPrepare;
    return typeof prepare === 'function' ? prepare.apply(this, args) : undefined;
  },
};
