#!/usr/bin/env node
// Builds IMT's helper processes and puts them where Tauri bundles them as sidecars
// (src-tauri/binaries/<name>-<target-triple>[.exe], listed in src-tauri/tauri.sidecars.conf.json):
//   imt-ai-worker    the AI assistant's engine process
//   imt-plugins      serves every built-in plugin, each in its own sandboxed process (ADR 0017)
//   imt-store-worker hosts every window's DuckDB store and full-text index (ADR 0014)
// At run time IMT finds them next to its executable. The store worker links libduckdb (duckdb.dll / libduckdb.so /
// libduckdb.dylib): it is copied from DUCKDB_LIB_DIR into src-tauri/binaries/lib/; when DUCKDB_LIB_DIR is not set,
// crates/imt-store-duck/scripts/fetch-libduckdb.sh fetches the pinned, checksummed library into <target>/libduckdb
// first. The installers ship src-tauri/binaries/lib/ as the resource folder `lib/` (tauri.sidecars.conf.json).
//
// Run by the release build (`npx tauri build --config src-tauri/tauri.sidecars.conf.json` chains it after the
// UI build). Tauri passes the target in TAURI_ENV_TARGET_TRIPLE; `universal-apple-darwin` builds both macOS
// architectures and joins them with lipo.
// Usage: node scripts/build-sidecars.mjs [--target <triple>] [--debug]
// IMT_CARGO_PROFILE names another cargo profile (e.g. `dist` for releases); it must match the app's `--profile`.
import { execFileSync } from 'node:child_process';
import { chmodSync, copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { fetchLibduckdb } from './libduckdb.mjs';

export const SIDECARS = ['imt-ai-worker', 'imt-plugins', 'imt-store-worker'];

/** The file name of the prebuilt libduckdb for a target triple. */
export function libduckdbFile(target) {
  if (target.includes('-windows-')) return 'duckdb.dll';
  if (target.includes('-apple-')) return 'libduckdb.dylib';
  return 'libduckdb.so';
}

/**
 * What to build and where cargo puts it. A build for the host itself runs without `--target`, into
 * `<targetDir>/<profile>`, where `tauri build` (without `--target`) builds the app: with `--target`, cargo
 * used `<targetDir>/<triple>/<profile>` and compiled every dependency a second time. Other targets, and both
 * halves of `universal-apple-darwin` (which Tauri also builds per triple), keep `--target` and their folder.
 */
export function sidecarPlan({ target, host, profile, targetDir }) {
  const native = target === host;
  const triples = target === 'universal-apple-darwin' ? ['aarch64-apple-darwin', 'x86_64-apple-darwin'] : [target];
  const select = SIDECARS.flatMap((s) => ['-p', s, '--bin', s]);
  const release = profile === 'release' ? ['--release'] : profile === 'debug' ? [] : ['--profile', profile];
  const builds = native ? [['build', ...select, ...release]] : triples.map((t) => ['build', ...select, '--target', t, ...release]);
  const ext = target.includes('-windows-') ? '.exe' : '';
  const binaries = SIDECARS.map((name) => ({
    name,
    file: `${name}-${target}${ext}`,
    built: native ? [join(targetDir, profile, `${name}${ext}`)] : triples.map((t) => join(targetDir, t, profile, `${name}${ext}`)),
  }));
  return { builds, binaries, ext };
}

async function main() {
  const app = join(dirname(fileURLToPath(import.meta.url)), '..');
  const root = join(app, '..');
  const args = process.argv.slice(2);
  const flag = (name) => {
    const i = args.indexOf(name);
    return i >= 0 ? args[i + 1] : undefined;
  };
  const profile = args.includes('--debug') ? 'debug' : (process.env.IMT_CARGO_PROFILE ?? 'release');
  const host = /^host: (.+)$/m.exec(execFileSync('rustc', ['-vV'], { encoding: 'utf8' }))?.[1]?.trim();
  const target = flag('--target') ?? process.env.TAURI_ENV_TARGET_TRIPLE ?? host;
  if (!target) throw new Error('cannot tell the target: pass --target <triple>');

  const targetDir = process.env.CARGO_TARGET_DIR ?? join(root, 'target');
  const outDir = join(app, 'src-tauri', 'binaries');
  mkdirSync(outDir, { recursive: true });
  const run = (cmd, argv) => {
    console.log(`${cmd} ${argv.join(' ')}`);
    execFileSync(cmd, argv, { cwd: root, stdio: 'inherit' });
  };

  // The store worker links libduckdb: fetch the pinned library unless the caller already did (CI, the docs).
  if (!process.env.DUCKDB_LIB_DIR) {
    // In Node, not the shell script: Windows has no bash.
    const script = join(root, 'crates', 'imt-store-duck', 'scripts', 'fetch-libduckdb.sh');
    console.log('DUCKDB_LIB_DIR is not set: fetching the pinned libduckdb');
    Object.assign(process.env, await fetchLibduckdb(script, join(targetDir, 'libduckdb')));
  }
  const { builds, binaries, ext } = sidecarPlan({ target, host, profile, targetDir });
  for (const argv of builds) run('cargo', argv);
  for (const { file, built } of binaries) {
    const out = join(outDir, file);
    if (built.length > 1) run('lipo', ['-create', '-output', out, ...built]);
    else copyFileSync(built[0], out);
    if (!ext) chmodSync(out, 0o755);
    console.log(`sidecar: ${out}`);
  }
  // The store worker's database library, shipped as the resource folder `lib/`.
  const libDir = process.env.DUCKDB_LIB_DIR;
  const lib = libduckdbFile(target);
  const libOut = join(outDir, 'lib');
  mkdirSync(libOut, { recursive: true });
  copyFileSync(join(libDir, lib), join(libOut, lib));
  console.log(`library: ${join(libOut, lib)}`);
  // And next to the built worker, where an app run straight from the target folder (development, the end-to-end
  // builds) finds it.
  for (const built of binaries.find((b) => b.name === 'imt-store-worker').built) {
    copyFileSync(join(libDir, lib), join(dirname(built), lib));
    console.log(`library: ${join(dirname(built), lib)}`);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
