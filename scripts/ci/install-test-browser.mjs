#!/usr/bin/env node
// Installs the pinned Chrome for Testing build (PINNED_VERSION and SUMS in app/src-tauri/src/browser/runtime.rs, ADR 0043,
// the same build the app itself may download) into a folder, so tests never launch an installed Chrome/Edge/Chromium
// (Paddock rule 7). The archive is verified against its SHA-256 before it is extracted.
//
//   node scripts/ci/install-test-browser.mjs [dir]      (default: $DEBRIEF_TEST_BROWSER_DIR or ~/.cache/debrief-test-browser)
//
// Prints `CHROMIUM_TEST_PATH=<executable>`; with $GITHUB_ENV set it also appends that line there for the later steps.
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { appendFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const RUNTIME_RS = fileURLToPath(new URL('../../app/src-tauri/src/browser/runtime.rs', import.meta.url));

/** The pin from runtime.rs: `{version, base, sums: {platform: sha256}}` (pure; throws when the source changed shape). */
export function parsePin(rs) {
  const version = rs.match(/PINNED_VERSION: &str = "([^"]+)"/)?.[1];
  const base = rs.match(/const BASE: &str = "([^"]+)"/)?.[1];
  const sums = {};
  for (const m of rs.matchAll(/\("([a-z0-9-]+)",\s*"([0-9a-f]{64})"\)/g)) sums[m[1]] = m[2];
  if (!version || !base || !Object.keys(sums).length) throw new Error('could not read the Chromium pin from runtime.rs');
  return { version, base, sums };
}

/** Chrome for Testing platform id of a Node platform/arch, or throws when there is no pinned build. */
export function platformId(platform = process.platform, arch = process.arch) {
  const id = { 'linux/x64': 'linux64', 'darwin/arm64': 'mac-arm64', 'darwin/x64': 'mac-x64', 'win32/x64': 'win64' }[`${platform}/${arch}`];
  if (!id) throw new Error(`no pinned Chromium for ${platform}/${arch}`);
  return id;
}

/** Download URL of a platform's archive. */
export function urlFor({ version, base }, id) {
  return `${base}/${version}/${id}/chrome-${id}.zip`;
}

/** The browser executable inside the extracted folder. */
export function executableIn(dir, id) {
  const root = join(dir, `chrome-${id}`);
  if (id === 'win64') return join(root, 'chrome.exe');
  if (id.startsWith('mac')) return join(root, 'Google Chrome for Testing.app', 'Contents', 'MacOS', 'Google Chrome for Testing');
  return join(root, 'chrome');
}

async function main(argv) {
  const dir = argv[0] ?? process.env.DEBRIEF_TEST_BROWSER_DIR ?? join(homedir(), '.cache', 'debrief-test-browser');
  const pin = parsePin(readFileSync(RUNTIME_RS, 'utf8'));
  const id = platformId();
  const sha = pin.sums[id];
  if (!sha) throw new Error(`no checksum for ${id} in runtime.rs`);
  const exe = executableIn(dir, id);
  const stamp = join(dir, '.pinned-version');
  const have = existsSync(stamp) && readFileSync(stamp, 'utf8').trim() === pin.version && existsSync(exe);
  if (!have) {
    const url = urlFor(pin, id);
    const work = mkdtempSync(join(tmpdir(), 'debrief-cft-'));
    try {
      const res = await fetch(url);
      if (!res.ok) throw new Error(`${url}: ${res.status}`);
      const bytes = Buffer.from(await res.arrayBuffer());
      const got = createHash('sha256').update(bytes).digest('hex');
      if (got !== sha) throw new Error(`checksum mismatch for ${url}: ${got}`);
      const zip = join(work, 'cft.zip');
      writeFileSync(zip, bytes);
      rmSync(dir, { recursive: true, force: true });
      mkdirSync(dir, { recursive: true });
      if (process.platform === 'win32') execFileSync('tar', ['-xf', zip, '-C', dir]);
      else execFileSync('unzip', ['-q', zip, '-d', dir]);
      writeFileSync(stamp, pin.version);
    } finally {
      rmSync(work, { recursive: true, force: true });
    }
  }
  if (!existsSync(exe)) throw new Error(`the archive did not contain ${exe}`);
  const line = `CHROMIUM_TEST_PATH=${exe}`;
  console.log(line);
  if (process.env.GITHUB_ENV) appendFileSync(process.env.GITHUB_ENV, `${line}\n`);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  main(process.argv.slice(2)).catch((e) => {
    console.error(`install-test-browser: ${e.message}`);
    process.exit(1);
  });
}
