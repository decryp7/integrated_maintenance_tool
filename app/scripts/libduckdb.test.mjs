// Run: node --test scripts/libduckdb.test.mjs
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';
import { assetFor, parsePins } from './libduckdb.mjs';

const script = readFileSync(join(import.meta.dirname, '..', '..', 'crates', 'imt-store-duck', 'scripts', 'fetch-libduckdb.sh'), 'utf8');

test('the pins come from fetch-libduckdb.sh, the one list CI and the docs use too', () => {
  const pins = parsePins(script);
  assert.match(pins.version, /^\d+\.\d+\.\d+$/);
  assert.deepEqual(
    pins.assets.map((a) => a.asset).sort(),
    ['libduckdb-linux-amd64.zip', 'libduckdb-linux-arm64.zip', 'libduckdb-osx-universal.zip', 'libduckdb-windows-amd64.zip', 'libduckdb-windows-arm64.zip'],
  );
  assert.ok(pins.assets.every((a) => /^[0-9a-f]{64}$/.test(a.sha)));
});

test('each platform gets its own asset; an unpinned one is an error that names the pinned ones', () => {
  const pins = parsePins(script);
  assert.equal(assetFor(pins, 'win32', 'x64').asset, 'libduckdb-windows-amd64.zip');
  assert.equal(assetFor(pins, 'win32', 'arm64').asset, 'libduckdb-windows-arm64.zip');
  assert.equal(assetFor(pins, 'linux', 'x64').asset, 'libduckdb-linux-amd64.zip');
  assert.equal(assetFor(pins, 'darwin', 'arm64').asset, 'libduckdb-osx-universal.zip');
  assert.equal(assetFor(pins, 'darwin', 'x64').asset, 'libduckdb-osx-universal.zip');
  assert.throws(() => assetFor(pins, 'freebsd', 'x64'), /no pinned libduckdb for freebsd-x64 \(pinned: libduckdb-/);
  assert.throws(() => parsePins('nothing here'), /no VERSION/);
});
