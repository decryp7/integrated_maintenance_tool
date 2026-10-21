import { browser, expect } from '@wdio/globals';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { waitForShell } from '../support/app';
import { LIVE_DIR } from '../support/paths';

// What the app's live search probe (app/src-tauri/src/browser/live_probe.rs) writes: one line per engine, from the
// real page webview, the real collector script and the real Rust rules.
interface PageDiagnostics {
  host: string;
  path: string;
  title: string;
  readyState: string;
  quietMs: number | null;
  totalLinks: number;
  links: number;
  linksInRegion: number;
  regionFound: boolean;
  kind: string;
  /** host/path?parameter-names of the links in the results region (no values). */
  regionLinks: string[];
}
interface EngineReport {
  engine: string;
  label: string;
  verdict: 'pass' | 'warn' | 'fail';
  note: string;
  links: number;
  resultSites: string[];
  looks: number;
  ready: boolean;
  elapsedMs: number;
  page: PageDiagnostics | null;
  screenshot: string | null;
  screenshotError: string | null;
}
interface Report {
  query: string;
  platform: string;
  done: boolean;
  engines: EngineReport[];
}

const REPORT = join(LIVE_DIR, 'live-search.json');
// IMT_E2E_LIVE_SEARCH_ENGINES (comma-separated ids) limits a local run to some engines; the app reads it too.
const ENGINES = (process.env['IMT_E2E_LIVE_SEARCH_ENGINES'] || 'google,duckDuckGo,bing,stackOverflow')
  .split(',')
  .map((e) => e.trim())
  .filter(Boolean);

function read(): Report | null {
  if (!existsSync(REPORT)) return null;
  try {
    return JSON.parse(readFileSync(REPORT, 'utf8')) as Report;
  } catch {
    return null; // being replaced
  }
}

function line(e: EngineReport): string {
  const p = e.page;
  const page = p
    ? `${p.host}${p.path} "${p.title}" ${p.kind}, readyState ${p.readyState}, quiet ${p.quietMs ?? '-'} ms, ` +
      `${p.totalLinks} links (${p.linksInRegion} in region, region ${p.regionFound ? 'found' : 'missing'})`
    : 'no page';
  return `${e.label}: ${e.verdict.toUpperCase()} ${e.note}; ${e.looks} looks in ${e.elapsedMs} ms (ready: ${e.ready}); ${page}; sites: ${e.resultSites.join(', ') || '-'}; screenshot: ${e.screenshot ?? e.screenshotError}`;
}

describe('live: research reads the results page of every search engine', () => {
  let report: Report;

  before(async () => {
    await waitForShell();
    await browser.waitUntil(() => read()?.done === true, {
      timeout: 270_000,
      interval: 1_000,
      timeoutMsg: `the app wrote no complete report to ${REPORT} (see IMT's log: "live search probe")`,
    });
    report = read()!;
    console.log(`Live search for "${report.query}" on ${report.platform}:\n${report.engines.map(line).join('\n')}`);
  });

  for (const id of ENGINES) {
    it(`${id}: at least 3 result links, or a recognised hand-off`, () => {
      const e = report.engines.find((x) => x.engine === id);
      if (!e) throw new Error(`${id} is missing from the report`);
      // A consent, human-check or redirect page is recognised and handed to the user: a warning, not a pass.
      if (e.verdict === 'warn') console.warn(`WARNING ${line(e)}`);
      if (e.verdict === 'fail')
        throw new Error(`${line(e)}\nlinks in the results region: ${e.page?.regionLinks.join(' | ') || '-'}`);
      expect(e.links).toBeGreaterThanOrEqual(e.verdict === 'pass' ? 3 : 0);
    });
  }
});
