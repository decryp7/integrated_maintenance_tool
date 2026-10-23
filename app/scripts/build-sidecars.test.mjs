// Run: node --test scripts/build-sidecars.test.mjs
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { test } from 'node:test';
import { adhocSign, libduckdbFile, sidecarPlan } from './build-sidecars.mjs';

const select = [
  '-p',
  'debrief-ai-worker',
  '--bin',
  'debrief-ai-worker',
  '-p',
  'debrief-plugins',
  '--bin',
  'debrief-plugins',
  '-p',
  'debrief-store-worker',
  '--bin',
  'debrief-store-worker',
];
const host = 'x86_64-pc-windows-msvc';
const targetDir = join('ci', 'target');

test('a host build runs without --target and reads target/<profile>, where tauri build puts its own', () => {
  const plan = sidecarPlan({ target: host, host, profile: 'release', targetDir });
  assert.deepEqual(plan.builds, [['build', ...select, '--release']]);
  assert.deepEqual(
    plan.binaries.map((b) => [b.file, b.built]),
    [
      ['debrief-ai-worker-x86_64-pc-windows-msvc.exe', [join(targetDir, 'release', 'debrief-ai-worker.exe')]],
      ['debrief-plugins-x86_64-pc-windows-msvc.exe', [join(targetDir, 'release', 'debrief-plugins.exe')]],
      ['debrief-store-worker-x86_64-pc-windows-msvc.exe', [join(targetDir, 'release', 'debrief-store-worker.exe')]],
    ],
  );
});

test('a debug host build has no --release and reads target/debug', () => {
  const plan = sidecarPlan({ target: 'x86_64-unknown-linux-gnu', host: 'x86_64-unknown-linux-gnu', profile: 'debug', targetDir });
  assert.deepEqual(plan.builds, [['build', ...select]]);
  assert.equal(plan.ext, '');
  assert.deepEqual(plan.binaries[0].built, [join(targetDir, 'debug', 'debrief-ai-worker')]);
});

test('a custom profile (dist) passes --profile and reads target/<profile>', () => {
  const plan = sidecarPlan({ target: host, host, profile: 'dist', targetDir });
  assert.deepEqual(plan.builds, [['build', ...select, '--profile', 'dist']]);
  assert.deepEqual(plan.binaries[0].built, [join(targetDir, 'dist', 'debrief-ai-worker.exe')]);
});

test('a cross build keeps --target and its own folder', () => {
  const plan = sidecarPlan({ target: 'aarch64-pc-windows-msvc', host, profile: 'release', targetDir });
  assert.deepEqual(plan.builds, [['build', ...select, '--target', 'aarch64-pc-windows-msvc', '--release']]);
  assert.deepEqual(plan.binaries[1].built, [join(targetDir, 'aarch64-pc-windows-msvc', 'release', 'debrief-plugins.exe')]);
  assert.equal(plan.binaries[1].file, 'debrief-plugins-aarch64-pc-windows-msvc.exe');
});

test('universal macOS builds both architectures with --target, like tauri build does, for lipo', () => {
  const plan = sidecarPlan({ target: 'universal-apple-darwin', host: 'aarch64-apple-darwin', profile: 'release', targetDir });
  assert.deepEqual(plan.builds, [
    ['build', ...select, '--target', 'aarch64-apple-darwin', '--release'],
    ['build', ...select, '--target', 'x86_64-apple-darwin', '--release'],
  ]);
  assert.equal(plan.binaries[0].file, 'debrief-ai-worker-universal-apple-darwin');
  assert.deepEqual(plan.binaries[0].built, [
    join(targetDir, 'aarch64-apple-darwin', 'release', 'debrief-ai-worker'),
    join(targetDir, 'x86_64-apple-darwin', 'release', 'debrief-ai-worker'),
  ]);
});

test('an unknown host never drops --target', () => {
  const plan = sidecarPlan({ target: 'x86_64-unknown-linux-gnu', host: undefined, profile: 'release', targetDir });
  assert.deepEqual(plan.builds, [['build', ...select, '--target', 'x86_64-unknown-linux-gnu', '--release']]);
});

test('the store worker ships the prebuilt libduckdb of its platform', () => {
  assert.equal(libduckdbFile('x86_64-pc-windows-msvc'), 'duckdb.dll');
  assert.equal(libduckdbFile('x86_64-unknown-linux-gnu'), 'libduckdb.so');
  assert.equal(libduckdbFile('universal-apple-darwin'), 'libduckdb.dylib');
});

test('a bundled dylib is re-signed ad hoc on macOS only', () => {
  const calls = [];
  const exec = (cmd, args) => calls.push([cmd, ...args]);
  assert.equal(adhocSign('/x/libduckdb.dylib', exec, 'darwin'), true);
  assert.deepEqual(calls, [['codesign', '--force', '--sign', '-', '--timestamp=none', '/x/libduckdb.dylib']]);
  assert.equal(adhocSign('/x/libduckdb.so', exec, 'linux'), false);
  assert.equal(calls.length, 1);
});
