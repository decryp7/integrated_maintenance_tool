#!/usr/bin/env node
// Builds IMT's helper processes and puts them where Tauri bundles them as sidecars
// (src-tauri/binaries/<name>-<target-triple>[.exe], listed in src-tauri/tauri.sidecars.conf.json):
//   imt-ai-worker    the AI assistant's engine process
//   imt-plugin-host  serves built-in plugins out of process when they run isolated
// At run time IMT finds them next to its executable.
//
// Run by the release build (`npx tauri build --config src-tauri/tauri.sidecars.conf.json` chains it after the
// UI build). Tauri passes the target in TAURI_ENV_TARGET_TRIPLE; `universal-apple-darwin` builds both macOS
// architectures and joins them with lipo.
// Usage: node scripts/build-sidecars.mjs [--target <triple>] [--debug]
import { execFileSync } from 'node:child_process';
import { chmodSync, copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const SIDECARS = ['imt-ai-worker', 'imt-plugin-host'];
const app = join(dirname(fileURLToPath(import.meta.url)), '..');
const root = join(app, '..');

const args = process.argv.slice(2);
const flag = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};
const profile = args.includes('--debug') ? 'debug' : 'release';
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

const triples = target === 'universal-apple-darwin' ? ['aarch64-apple-darwin', 'x86_64-apple-darwin'] : [target];
for (const t of triples) {
  const select = SIDECARS.flatMap((s) => ['-p', s, '--bin', s]);
  run('cargo', ['build', ...select, '--target', t, ...(profile === 'release' ? ['--release'] : [])]);
}
const ext = target.includes('-windows-') ? '.exe' : '';
for (const name of SIDECARS) {
  const out = join(outDir, `${name}-${target}${ext}`);
  const built = triples.map((t) => join(targetDir, t, profile, `${name}${ext}`));
  if (built.length > 1) run('lipo', ['-create', '-output', out, ...built]);
  else copyFileSync(built[0], out);
  if (!ext) chmodSync(out, 0o755);
  console.log(`sidecar: ${out}`);
}
