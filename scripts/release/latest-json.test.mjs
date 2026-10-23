// Run: node --test scripts/release
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { archOf, buildManifest, cleanNotes, MAX_NOTES, platformKeys } from './latest-json.mjs';

const base = { version: 'v1.2.0', notes: 'Fixes\r\n- grid\u0007', pubDate: '2026-10-17T12:00:00Z', baseUrl: 'https://h/o/r/releases/download/v1.2.0/' };
const file = (name, signature = 'c2ln') => ({ name, size: 42, signature });

test('platform keys follow the updater targets', () => {
  assert.deepEqual(platformKeys('DEBRIEF_1.2.0_x64-setup.exe'), ['windows-x86_64-nsis', 'windows-x86_64']);
  // Windows ships only the NSIS installer: an MSI is not an updater artifact.
  assert.equal(platformKeys('DEBRIEF_1.2.0_x64_en-US.msi'), null);
  assert.deepEqual(platformKeys('DEBRIEF_1.2.0_amd64.deb'), ['linux-x86_64-deb']);
  assert.deepEqual(platformKeys('Debrief-1.2.0-1.x86_64.rpm'), ['linux-x86_64-rpm']);
  assert.deepEqual(platformKeys('Debrief.app.tar.gz'), ['darwin-aarch64-app', 'darwin-aarch64']);
  assert.deepEqual(platformKeys('Debrief.app.tar.gz', 'x86_64'), ['darwin-x86_64-app', 'darwin-x86_64']);
  assert.equal(platformKeys('DEBRIEF_1.2.0_aarch64.dmg'), null);
  assert.equal(platformKeys('RELEASE_NOTES.md'), null);
  assert.equal(archOf('DEBRIEF_1.2.0_arm64-setup.exe'), 'aarch64');
  assert.equal(archOf('Debrief.app.tar.gz'), null);
});

test('builds the manifest from signed artifacts only, NSIS for Windows', () => {
  const m = buildManifest({ ...base, files: [file('DEBRIEF_1.2.0_x64_en-US.msi'), file('DEBRIEF_1.2.0_x64-setup.exe'), file('DEBRIEF_1.2.0_amd64.deb', null)] });
  assert.equal(m.version, '1.2.0');
  assert.equal(m.notes, 'Fixes\n- grid');
  assert.equal(m.pub_date, '2026-10-17T12:00:00Z');
  assert.deepEqual(Object.keys(m.platforms).sort(), ['windows-x86_64', 'windows-x86_64-nsis']);
  assert.equal(m.platforms['windows-x86_64'].url, 'https://h/o/r/releases/download/v1.2.0/DEBRIEF_1.2.0_x64-setup.exe');
  assert.equal(m.platforms['windows-x86_64'].size, 42);
  assert.equal(m.platforms['windows-x86_64-nsis'].signature, 'c2ln');
});

test('refuses bad input', () => {
  assert.throws(() => buildManifest({ ...base, files: [] }), /no signed updater artifacts/);
  assert.throws(() => buildManifest({ ...base, version: 'latest', files: [file('a-setup.exe')] }), /semantic version/);
  assert.throws(() => buildManifest({ ...base, baseUrl: 'http://h', files: [file('a-setup.exe')] }), /https/);
  assert.throws(() => buildManifest({ ...base, files: [file('a-setup.exe'), file('a-setup.exe')] }), /unique/);
});

test('notes are bounded plain text', () => {
  assert.equal(cleanNotes('x'.repeat(MAX_NOTES + 5)).length, MAX_NOTES + 1);
  assert.equal(cleanNotes('  a\tb\u001b  '), 'a\tb');
});

test('file names are URL-encoded', () => {
  const m = buildManifest({ ...base, files: [file('Debrief 1.2.0_x64-setup.exe')] });
  assert.match(m.platforms['windows-x86_64'].url, /Debrief%201\.2\.0_x64-setup\.exe$/);
});
