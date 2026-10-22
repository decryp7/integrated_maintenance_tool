// Fetches the official prebuilt libduckdb the store worker links, without a shell: Windows has no bash. The pins
// (version, release assets and their SHA-256) stay in one place, crates/imt-store-duck/scripts/fetch-libduckdb.sh,
// which CI and the docs run directly; this module reads them from it. The archive is checked against its pin before
// it is unpacked (Windows 10+ `tar`, which reads zip; `unzip` elsewhere) into a fresh folder of the cache.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/** The pins of fetch-libduckdb.sh: its `VERSION=` and every `ASSET=…; SHA=…` line. */
export function parsePins(script) {
  const version = /^VERSION=(\S+)$/m.exec(script)?.[1];
  if (!version) throw new Error('fetch-libduckdb.sh names no VERSION');
  const assets = [...script.matchAll(/ASSET=(libduckdb-[\w.-]+\.zip); SHA=([0-9a-f]{64})/g)].map((m) => ({ asset: m[1], sha: m[2] }));
  if (assets.length === 0) throw new Error('fetch-libduckdb.sh pins no assets');
  return { version, assets };
}

/** The pinned asset for a Node `platform` and `arch`, or an error naming what is pinned. */
export function assetFor(pins, platform, arch) {
  const cpu = { x64: 'amd64', arm64: 'arm64' }[arch];
  const name =
    platform === 'darwin' ? 'libduckdb-osx-universal.zip' : platform === 'win32' ? `libduckdb-windows-${cpu}.zip` : platform === 'linux' ? `libduckdb-linux-${cpu}.zip` : null;
  const pin = pins.assets.find((a) => a.asset === name);
  if (!pin) throw new Error(`no pinned libduckdb for ${platform}-${arch} (pinned: ${pins.assets.map((a) => a.asset).join(', ')})`);
  return pin;
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

/**
 * The library folders for the build (`DUCKDB_LIB_DIR`, `DUCKDB_INCLUDE_DIR`), fetching and verifying the pinned
 * archive into `cacheDir` first when it is not there yet.
 */
export async function fetchLibduckdb(scriptPath, cacheDir, platform = process.platform, arch = process.arch) {
  const pins = parsePins(readFileSync(scriptPath, 'utf8'));
  const { asset, sha } = assetFor(pins, platform, arch);
  const dir = join(cacheDir, pins.version, asset.replace(/\.zip$/, ''));
  if (!existsSync(join(dir, '.verified'))) {
    const url = `https://github.com/duckdb/duckdb/releases/download/v${pins.version}/${asset}`;
    console.error(`fetching ${url}`);
    const { res, bytes } = await download(url, { redirect: 'follow' });
    if (!res.ok) throw new Error(`downloading ${asset} failed: HTTP ${res.status}`);
    const got = createHash('sha256').update(bytes).digest('hex');
    if (got !== sha) throw new Error(`SHA-256 mismatch for ${asset}: expected ${sha}, got ${got}`);
    mkdirSync(join(cacheDir, pins.version), { recursive: true });
    const zip = join(cacheDir, pins.version, asset);
    writeFileSync(zip, bytes);
    // A fresh, empty folder; only the library, its import library and the headers are used.
    rmSync(dir, { recursive: true, force: true });
    mkdirSync(dir, { recursive: true });
    // Windows' own bsdtar (System32), which reads zip; a `tar` earlier on PATH (Git's GNU tar) takes `C:` for a host.
    const tar = join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'tar.exe');
    if (platform === 'win32') execFileSync(tar, ['-xf', zip, '-C', dir], { stdio: 'inherit' });
    else execFileSync('unzip', ['-q', '-o', zip, '-d', dir], { stdio: 'inherit' });
    writeFileSync(join(dir, '.verified'), '');
  }
  return { DUCKDB_LIB_DIR: dir, DUCKDB_INCLUDE_DIR: dir };
}

// `node app/scripts/libduckdb.mjs [cache-dir]` (for cargo builds outside `tauri build`, e.g. on Windows without bash):
// fetches the library and prints the lines that set the build's variables, for PowerShell on Windows, sh elsewhere.
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
  const cache = process.argv[2] ?? join(process.env.CARGO_TARGET_DIR ?? join(root, 'target'), 'libduckdb');
  const env = await fetchLibduckdb(join(root, 'crates', 'imt-store-duck', 'scripts', 'fetch-libduckdb.sh'), cache);
  const lines =
    process.platform === 'win32'
      ? [`$env:DUCKDB_LIB_DIR = '${env.DUCKDB_LIB_DIR}'`, `$env:DUCKDB_INCLUDE_DIR = '${env.DUCKDB_INCLUDE_DIR}'`, `$env:PATH = '${env.DUCKDB_LIB_DIR};' + $env:PATH`]
      : [`export DUCKDB_LIB_DIR='${env.DUCKDB_LIB_DIR}' DUCKDB_INCLUDE_DIR='${env.DUCKDB_INCLUDE_DIR}'`];
  console.log(lines.join('\n'));
}
