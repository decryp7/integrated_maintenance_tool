// Run: node --test scripts/build-sidecars.test.mjs
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { test } from 'node:test';
import { sidecarPlan } from './build-sidecars.mjs';

const select = ['-p', 'imt-ai-worker', '--bin', 'imt-ai-worker', '-p', 'imt-plugin-host', '--bin', 'imt-plugin-host'];
const host = 'x86_64-pc-windows-msvc';
const targetDir = join('ci', 'target');

test('a host build runs without --target and reads target/<profile>, where tauri build puts its own', () => {
  const plan = sidecarPlan({ target: host, host, profile: 'release', targetDir });
  assert.deepEqual(plan.builds, [['build', ...select, '--release']]);
  assert.deepEqual(
    plan.binaries.map((b) => [b.file, b.built]),
    [
      ['imt-ai-worker-x86_64-pc-windows-msvc.exe', [join(targetDir, 'release', 'imt-ai-worker.exe')]],
      ['imt-plugin-host-x86_64-pc-windows-msvc.exe', [join(targetDir, 'release', 'imt-plugin-host.exe')]],
    ],
  );
});

test('a debug host build has no --release and reads target/debug', () => {
  const plan = sidecarPlan({ target: 'x86_64-unknown-linux-gnu', host: 'x86_64-unknown-linux-gnu', profile: 'debug', targetDir });
  assert.deepEqual(plan.builds, [['build', ...select]]);
  assert.equal(plan.ext, '');
  assert.deepEqual(plan.binaries[0].built, [join(targetDir, 'debug', 'imt-ai-worker')]);
});

test('a cross build keeps --target and its own folder', () => {
  const plan = sidecarPlan({ target: 'aarch64-pc-windows-msvc', host, profile: 'release', targetDir });
  assert.deepEqual(plan.builds, [['build', ...select, '--target', 'aarch64-pc-windows-msvc', '--release']]);
  assert.deepEqual(plan.binaries[1].built, [join(targetDir, 'aarch64-pc-windows-msvc', 'release', 'imt-plugin-host.exe')]);
  assert.equal(plan.binaries[1].file, 'imt-plugin-host-aarch64-pc-windows-msvc.exe');
});

test('universal macOS builds both architectures with --target, like tauri build does, for lipo', () => {
  const plan = sidecarPlan({ target: 'universal-apple-darwin', host: 'aarch64-apple-darwin', profile: 'release', targetDir });
  assert.deepEqual(plan.builds, [
    ['build', ...select, '--target', 'aarch64-apple-darwin', '--release'],
    ['build', ...select, '--target', 'x86_64-apple-darwin', '--release'],
  ]);
  assert.equal(plan.binaries[0].file, 'imt-ai-worker-universal-apple-darwin');
  assert.deepEqual(plan.binaries[0].built, [
    join(targetDir, 'aarch64-apple-darwin', 'release', 'imt-ai-worker'),
    join(targetDir, 'x86_64-apple-darwin', 'release', 'imt-ai-worker'),
  ]);
});

test('an unknown host never drops --target', () => {
  const plan = sidecarPlan({ target: 'x86_64-unknown-linux-gnu', host: undefined, profile: 'release', targetDir });
  assert.deepEqual(plan.builds, [['build', ...select, '--target', 'x86_64-unknown-linux-gnu', '--release']]);
});
