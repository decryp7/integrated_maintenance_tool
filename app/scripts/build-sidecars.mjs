#!/usr/bin/env node
// Builds IMT's helper processes and puts them where Tauri bundles them as sidecars
// (src-tauri/binaries/<name>-<target-triple>[.exe], listed in src-tauri/tauri.sidecars.conf.json):
//   imt-ai-worker    the AI assistant's engine process
//   imt-plugins      serves every built-in plugin, each in its own sandboxed process (ADR 0017)
// At run time IMT finds them next to its executable.
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

export const SIDECARS = ['imt-ai-worker', 'imt-plugins'];

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

function main() {
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

  const { builds, binaries, ext } = sidecarPlan({ target, host, profile, targetDir });
  for (const argv of builds) run('cargo', argv);
  for (const { file, built } of binaries) {
    const out = join(outDir, file);
    if (built.length > 1) run('lipo', ['-create', '-output', out, ...built]);
    else copyFileSync(built[0], out);
    if (!ext) chmodSync(out, 0o755);
    console.log(`sidecar: ${out}`);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
