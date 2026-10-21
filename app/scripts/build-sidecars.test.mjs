// Run: node --test scripts/build-sidecars.test.mjs
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { test } from 'node:test';
import { libduckdbEnv, libduckdbFile, sidecarPlan } from './build-sidecars.mjs';

const select = [
  '-p',
  'imt-ai-worker',
  '--bin',
  'imt-ai-worker',
  '-p',
  'imt-plugins',
  '--bin',
  'imt-plugins',
  '-p',
  'imt-store-worker',
  '--bin',
  'imt-store-worker',
];
const host = 'x86_64-pc-windows-msvc';
const targetDir = join('ci', 'target');

test('a host build runs without --target and reads target/<profile>, where tauri build puts its own', () => {
  const plan = sidecarPlan({ target: host, host, profile: 'release', targetDir });
  assert.deepEqual(plan.builds, [['build', ...select, '--release']]);
  assert.deepEqual(
    plan.binaries.map((b) => [b.file, b.built]),
    [
      ['imt-ai-worker-x86_64-pc-windows-msvc.exe', [join(targetDir, 'release', 'imt-ai-worker.exe')]],
      ['imt-plugins-x86_64-pc-windows-msvc.exe', [join(targetDir, 'release', 'imt-plugins.exe')]],
      ['imt-store-worker-x86_64-pc-windows-msvc.exe', [join(targetDir, 'release', 'imt-store-worker.exe')]],
    ],
  );
});

test('a debug host build has no --release and reads target/debug', () => {
  const plan = sidecarPlan({ target: 'x86_64-unknown-linux-gnu', host: 'x86_64-unknown-linux-gnu', profile: 'debug', targetDir });
  assert.deepEqual(plan.builds, [['build', ...select]]);
  assert.equal(plan.ext, '');
  assert.deepEqual(plan.binaries[0].built, [join(targetDir, 'debug', 'imt-ai-worker')]);
});

test('a custom profile (dist) passes --profile and reads target/<profile>', () => {
  const plan = sidecarPlan({ target: host, host, profile: 'dist', targetDir });
  assert.deepEqual(plan.builds, [['build', ...select, '--profile', 'dist']]);
  assert.deepEqual(plan.binaries[0].built, [join(targetDir, 'dist', 'imt-ai-worker.exe')]);
});

test('a cross build keeps --target and its own folder', () => {
  const plan = sidecarPlan({ target: 'aarch64-pc-windows-msvc', host, profile: 'release', targetDir });
  assert.deepEqual(plan.builds, [['build', ...select, '--target', 'aarch64-pc-windows-msvc', '--release']]);
  assert.deepEqual(plan.binaries[1].built, [join(targetDir, 'aarch64-pc-windows-msvc', 'release', 'imt-plugins.exe')]);
  assert.equal(plan.binaries[1].file, 'imt-plugins-aarch64-pc-windows-msvc.exe');
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

test('the store worker ships the prebuilt libduckdb of its platform', () => {
  assert.equal(libduckdbFile('x86_64-pc-windows-msvc'), 'duckdb.dll');
  assert.equal(libduckdbFile('x86_64-unknown-linux-gnu'), 'libduckdb.so');
  assert.equal(libduckdbFile('universal-apple-darwin'), 'libduckdb.dylib');
});

test('the build takes the library folders fetch-libduckdb.sh exports', () => {
  const dir = '/Volumes/d/target/libduckdb/1.5.6/libduckdb-osx-universal';
  const mac = `export DUCKDB_LIB_DIR='${dir}' DUCKDB_INCLUDE_DIR='${dir}'\nexport DYLD_LIBRARY_PATH='${dir}':"\${DYLD_LIBRARY_PATH:-}"\n`;
  assert.deepEqual(libduckdbEnv(mac), {
    DUCKDB_LIB_DIR: dir,
    DUCKDB_INCLUDE_DIR: dir,
  });
  const win = "export DUCKDB_LIB_DIR='C:/t/libduckdb-windows-amd64' DUCKDB_INCLUDE_DIR='C:/t/libduckdb-windows-amd64'\n";
  assert.equal(libduckdbEnv(win).DUCKDB_LIB_DIR, 'C:/t/libduckdb-windows-amd64');
  assert.throws(() => libduckdbEnv('no pinned libduckdb for Haiku-x86_64'), /printed no DUCKDB_LIB_DIR/);
});
