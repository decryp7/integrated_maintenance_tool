#!/usr/bin/env node
// Downloads the Microsoft Edge WebDriver (msedgedriver) that matches the installed WebView2 runtime into
// e2e-tauri/.drivers/ (Windows only). tauri-driver drives WebView2 through it, and the two must share a
// version: a driver for another major version refuses the session.
//
//   node e2e-tauri/scripts/fetch-msedgedriver.mjs            # version read from the registry
//   node e2e-tauri/scripts/fetch-msedgedriver.mjs 155.0.4283.62
//
// The version comes from the WebView2 runtime's EdgeUpdate client key (machine-wide install first, then the
// per-user one). The archive is fetched from Microsoft's driver CDN and unpacked with the system `tar`
// (bsdtar, which reads zip files, ships with Windows 10 1803 and later).

import { execFileSync } from 'node:child_process';
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
  copyFileSync,
  readFileSync,
} from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { tmpdir } from 'node:os';

const WEBVIEW2_CLIENT = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';
const KEYS = [
  `HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\${WEBVIEW2_CLIENT}`,
  `HKLM\\SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\${WEBVIEW2_CLIENT}`,
  `HKCU\\Software\\Microsoft\\EdgeUpdate\\Clients\\${WEBVIEW2_CLIENT}`,
];
const VERSION = /^\d+\.\d+\.\d+\.\d+$/;

const here = dirname(fileURLToPath(import.meta.url));
const outDir = resolve(here, '..', '.drivers');
const exe = join(outDir, 'msedgedriver.exe');
const stamp = join(outDir, 'msedgedriver.version');

function webview2Version() {
  for (const key of KEYS) {
    try {
      const out = execFileSync('reg', ['query', key, '/v', 'pv'], {
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'ignore'],
      });
      const v = /pv\s+REG_SZ\s+(\S+)/.exec(out)?.[1];
      if (v && VERSION.test(v) && v !== '0.0.0.0') return v;
    } catch {
      // Key not present: try the next one.
    }
  }
  return null;
}

if (process.platform !== 'win32') {
  console.log(
    'msedgedriver is only needed on Windows (Linux uses WebKitWebDriver from webkit2gtk-driver).',
  );
  process.exit(0);
}

const version = process.argv[2] ?? webview2Version();
if (!version || !VERSION.test(version)) {
  console.error(
    'Could not find the WebView2 runtime version. Install the WebView2 runtime, or pass the version: fetch-msedgedriver.mjs <a.b.c.d>',
  );
  process.exit(1);
}
if (existsSync(exe) && existsSync(stamp) && readFileSync(stamp, 'utf8').trim() === version) {
  console.log(`msedgedriver ${version} is already in ${outDir}`);
  process.exit(0);
}

/** Downloads `url` (body included: a connection can drop mid-body), retrying network errors and HTTP 5xx with backoff. */
async function download(url, init, tries = 4) {
  for (let attempt = 1; ; attempt++) {
    try {
      const res = await fetch(url, init);
      if (res.status >= 500 && attempt < tries) throw new Error(`HTTP ${res.status}`);
      return { res, bytes: res.ok ? Buffer.from(await res.arrayBuffer()) : null };
    } catch (e) {
      if (attempt >= tries) throw e;
      console.error(`download of ${url} failed (${e.message ?? e}); retrying (${attempt}/${tries - 1})`);
      await new Promise((r) => setTimeout(r, 2_000 * attempt));
    }
  }
}

const url = `https://msedgedriver.microsoft.com/${version}/edgedriver_win64.zip`;
console.log(`Downloading ${url}`);
const { res, bytes } = await download(url, { signal: AbortSignal.timeout(120_000) });
if (!res.ok) {
  console.error(
    `Download failed: HTTP ${res.status}. Is there a driver for WebView2 ${version}? See https://developer.microsoft.com/microsoft-edge/tools/webdriver/`,
  );
  process.exit(1);
}
// Unpack in a fresh, empty folder and copy only the driver out of it.
const work = mkdtempSync(join(tmpdir(), 'imt-msedgedriver-'));
try {
  const zip = join(work, 'edgedriver.zip');
  writeFileSync(zip, bytes);
  const unpacked = join(work, 'x');
  mkdirSync(unpacked);
  // Windows' own bsdtar (System32), which reads zips: a GNU tar earlier on PATH (Git for Windows) cannot, and
  // takes `C:\…` for a remote host.
  const tar = process.env.SystemRoot ? join(process.env.SystemRoot, 'System32', 'tar.exe') : 'tar';
  execFileSync(tar, ['-xf', zip, '-C', unpacked], { stdio: 'inherit' });
  mkdirSync(outDir, { recursive: true });
  copyFileSync(join(unpacked, 'msedgedriver.exe'), exe);
  writeFileSync(stamp, `${version}\n`);
  console.log(`msedgedriver ${version} -> ${exe}`);
} finally {
  rmSync(work, { recursive: true, force: true });
}
