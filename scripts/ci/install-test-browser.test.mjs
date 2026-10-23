import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { executableIn, parsePin, platformId, urlFor } from './install-test-browser.mjs';

const rs = readFileSync(new URL('../../app/src-tauri/src/browser/runtime.rs', import.meta.url), 'utf8');

test('the pin is read from runtime.rs, with a checksum for every platform', () => {
  const pin = parsePin(rs);
  assert.match(pin.version, /^\d+\.\d+\.\d+\.\d+$/);
  for (const id of ['linux64', 'mac-arm64', 'mac-x64', 'win64']) assert.match(pin.sums[id], /^[0-9a-f]{64}$/);
  assert.equal(urlFor(pin, 'linux64'), `${pin.base}/${pin.version}/linux64/chrome-linux64.zip`);
});

test('a changed source shape fails loudly', () => {
  assert.throws(() => parsePin('fn main() {}'), /could not read/);
});

test('platform ids and executables', () => {
  assert.equal(platformId('darwin', 'arm64'), 'mac-arm64');
  assert.equal(platformId('win32', 'x64'), 'win64');
  assert.throws(() => platformId('linux', 'arm64'), /no pinned Chromium/);
  assert.ok(executableIn('/d', 'linux64').endsWith('chrome-linux64/chrome'));
  assert.ok(executableIn('/d', 'mac-arm64').includes('Google Chrome for Testing.app/Contents/MacOS'));
  assert.ok(executableIn('/d', 'win64').endsWith('chrome.exe'));
});
