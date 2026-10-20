// Run: node --test scripts/app-icon.test.mjs
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { parseIco, replaceIcoEntries, writeIco } from './app-icon.mjs';

const png = (tag) => Buffer.from(`\x89PNG-${tag}`);

test('round-trips entries, encoding 256 px as 0 in the directory', () => {
  const ico = writeIco([
    { size: 16, data: png('a') },
    { size: 256, data: png('big') },
  ]);
  assert.equal(ico[6], 16);
  assert.equal(ico[6 + 16], 0);
  const back = parseIco(ico);
  assert.deepEqual(
    back.map((e) => [e.size, e.data.toString()]),
    [
      [16, png('a').toString()],
      [256, png('big').toString()],
    ],
  );
});

test('replaces only the sizes it is given and keeps the order', () => {
  const ico = writeIco([16, 32, 48].map((size) => ({ size, data: png(`old${size}`) })));
  const out = parseIco(replaceIcoEntries(ico, new Map([[16, png('new16')], [24, png('unused')]])));
  assert.deepEqual(
    out.map((e) => e.data.toString()),
    [png('new16'), png('old32'), png('old48')].map(String),
  );
});

test('rejects files that are not icons or are truncated', () => {
  assert.throws(() => parseIco(Buffer.from('not an icon')), /not an \.ico/);
  const ico = writeIco([{ size: 16, data: png('a') }]);
  assert.throws(() => parseIco(ico.subarray(0, 10)), /truncated/);
  assert.throws(() => parseIco(ico.subarray(0, ico.length - 2)), /outside the file/);
});
