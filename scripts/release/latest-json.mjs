#!/usr/bin/env node
// Writes latest.json, the release information Debrief's updater reads (see docs/releasing.md and
// app/src-tauri/src/updater/manifest.rs), from the signed updater artifacts of a release.
//
//   node scripts/release/latest-json.mjs --dir artifacts --version 1.2.0 \
//     --base-url https://host/owner/repo/releases/download/v1.2.0 --notes artifacts/RELEASE_NOTES.md \
//     [--mac-arch aarch64] --out artifacts/latest.json
//
// Every installer with a `.sig` next to it (Tauri writes them when TAURI_SIGNING_PRIVATE_KEY is set and
// bundle.createUpdaterArtifacts is true) becomes a platform entry: { url, signature, size }. These are v2
// updater artifacts, the installers themselves (NSIS .exe, .deb, .rpm, .AppImage) and the macOS
// .app.tar.gz, never v1 zips.

import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { basename, join } from 'node:path';
import { pathToFileURL } from 'node:url';

/** Release notes beyond this are cut (the app bounds them too). */
export const MAX_NOTES = 20000;

/** `x86_64` or `aarch64` from an installer's file name, or null. */
export function archOf(name) {
  if (/(_|-|\.)(x64|amd64|x86_64)(_|-|\.)/i.test(name)) return 'x86_64';
  if (/(_|-|\.)(aarch64|arm64)(_|-|\.)/i.test(name)) return 'aarch64';
  return null;
}

/**
 * The platform keys an artifact serves ({os}-{arch}-{installer} first; the updater falls back to
 * {os}-{arch}). Null for files that are not updater artifacts.
 */
export function platformKeys(name, macArch = 'aarch64') {
  if (name.endsWith('.app.tar.gz')) return [`darwin-${archOf(name) ?? macArch}-app`, `darwin-${archOf(name) ?? macArch}`];
  const arch = archOf(name) ?? 'x86_64';
  if (name.endsWith('-setup.exe')) return [`windows-${arch}-nsis`, `windows-${arch}`];
  if (name.endsWith('.deb')) return [`linux-${arch}-deb`];
  if (name.endsWith('.rpm')) return [`linux-${arch}-rpm`];
  if (name.endsWith('.AppImage')) return [`linux-${arch}-appimage`, `linux-${arch}`];
  return null;
}

function walk(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(join(dir, e.name)) : [join(dir, e.name)]));
}

/** Plain-text notes: control characters other than new lines and tabs removed, bounded. */
export function cleanNotes(text) {
  // eslint-disable-next-line no-control-regex
  const clean = text.replace(/\r\n/g, '\n').replace(/[\u0000-\u0008\u000b-\u001f\u007f]/g, '').trim();
  return clean.length > MAX_NOTES ? `${clean.slice(0, MAX_NOTES)}…` : clean;
}

/**
 * Builds the manifest. `files` are { path, name, size, signature? }; only signed ones count. The first
 * artifact for a key wins (by name). Windows ships only the NSIS installer.
 */
export function buildManifest({ version, notes, pubDate, baseUrl, files, macArch = 'aarch64' }) {
  const v = version.replace(/^v/, '');
  if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$/.test(v)) throw new Error(`not a semantic version: ${version}`);
  if (!/^https:\/\//.test(baseUrl)) throw new Error('the base URL must use https');
  const names = new Set();
  for (const f of files) {
    if (names.has(f.name)) throw new Error(`two artifacts are named ${f.name}; release asset names must be unique`);
    names.add(f.name);
  }
  const platforms = {};
  const signed = files.filter((f) => f.signature && platformKeys(f.name, macArch)).sort((a, b) => a.name.localeCompare(b.name));
  for (const f of signed) {
    const entry = { url: `${baseUrl.replace(/\/+$/, '')}/${encodeURIComponent(f.name)}`, signature: f.signature.trim(), size: f.size };
    for (const key of platformKeys(f.name, macArch)) platforms[key] ??= entry;
  }
  if (Object.keys(platforms).length === 0) throw new Error('no signed updater artifacts (*.sig) found: are TAURI_SIGNING_PRIVATE_KEY and createUpdaterArtifacts set?');
  return { version: v, notes: cleanNotes(notes), pub_date: pubDate, platforms };
}

function args(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 2) {
    if (!argv[i].startsWith('--') || argv[i + 1] === undefined) throw new Error(`bad argument: ${argv[i]}`);
    out[argv[i].slice(2)] = argv[i + 1];
  }
  return out;
}

function main() {
  const a = args(process.argv.slice(2));
  for (const k of ['dir', 'version', 'base-url', 'out']) if (!a[k]) throw new Error(`missing --${k}`);
  const paths = walk(a.dir).filter((p) => !p.endsWith('.sig'));
  const files = paths.map((path) => {
    let signature;
    try {
      signature = readFileSync(`${path}.sig`, 'utf8');
    } catch {
      signature = undefined;
    }
    return { path, name: basename(path), size: statSync(path).size, signature };
  });
  const notes = a.notes ? readFileSync(a.notes, 'utf8') : '';
  const manifest = buildManifest({
    version: a.version,
    notes,
    pubDate: new Date().toISOString().replace(/\.\d{3}Z$/, 'Z'),
    baseUrl: a['base-url'],
    files: files.filter((f) => platformKeys(f.name, a['mac-arch'])),
    macArch: a['mac-arch'] ?? 'aarch64',
  });
  writeFileSync(a.out, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`wrote ${a.out}: ${Object.keys(manifest.platforms).join(', ')}`);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  try {
    main();
  } catch (e) {
    console.error(`latest-json: ${e.message}`);
    process.exit(1);
  }
}
