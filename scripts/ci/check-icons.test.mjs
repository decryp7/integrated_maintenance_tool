import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, cpSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { deflateSync } from 'node:zlib';
import { checkDir, decodePng, ICONS } from './check-icons.mjs';

const crc = (buf) => {
  let c, crcv = 0xffffffff;
  for (const byte of buf) {
    c = (crcv ^ byte) & 255;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    crcv = (crcv >>> 8) ^ c;
  }
  return (crcv ^ 0xffffffff) >>> 0;
};
const chunk = (type, data) => {
  const body = Buffer.concat([Buffer.from(type, 'latin1'), data]);
  const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
  const sum = Buffer.alloc(4); sum.writeUInt32BE(crc(body));
  return Buffer.concat([len, body, sum]);
};
/** An RGBA PNG of `size` pixels with opaque pixels where inside(x, y). */
function png(size, inside, colour = 6, background = 0) {
  const rows = [];
  for (let y = 0; y < size; y++) {
    const row = Buffer.alloc(1 + size * 4, background);
    row[0] = 0;
    for (let x = 0; x < size; x++) if (inside(x, y)) row.set([0x60, 0x30, 0xa0, 255], 1 + x * 4);
    rows.push(row);
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0); ihdr.writeUInt32BE(size, 4); ihdr[8] = 8; ihdr[9] = colour;
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(Buffer.concat(rows))), chunk('IEND', Buffer.alloc(0))]);
}
const dirWith = (make) => {
  const dir = mkdtempSync(join(tmpdir(), 'icons-'));
  for (const n of ICONS) writeFileSync(join(dir, n), make(n));
  return dir;
};

test('a centred, full-size icon passes', () => {
  assert.deepEqual(checkDir(dirWith(() => png(64, (x, y) => x >= 4 && x < 60 && y >= 4 && y < 60))), []);
});

test('artwork only in the top-left corner (the v1.0.0 placeholder) fails', () => {
  const found = checkDir(dirWith(() => png(64, (x, y) => x < 6 && y < 6)));
  assert.ok(found.some((p) => p.includes('icon.png') && p.includes('covers only')));
  assert.ok(found.some((p) => p.includes('32x32.png')));
});

test('artwork in the corner on an opaque white canvas fails', () => {
  const found = checkDir(dirWith(() => png(64, (x, y) => x < 6 && y < 6, 6, 255)));
  assert.ok(found.some((p) => p.includes('covers only')));
});

test('an off-centre icon fails', () => {
  const found = checkDir(dirWith(() => png(100, (x, y) => x >= 30 && y >= 30)));
  assert.ok(found.some((p) => p.includes('not centred')));
});

test('a blank icon and a missing file fail', () => {
  assert.ok(checkDir(dirWith(() => png(16, () => false))).every((p) => p.includes('empty')));
  assert.ok(checkDir(mkdtempSync(join(tmpdir(), 'icons-'))).length === ICONS.length);
});

test('unsupported PNG formats are reported clearly', () => {
  assert.throws(() => decodePng(png(8, () => true, 2)), /unsupported PNG/);
  assert.throws(() => decodePng(Buffer.from('nope')), /not a PNG/);
  assert.ok(checkDir(dirWith(() => png(8, () => true, 2)))[0].includes('unsupported PNG'));
});

test('the CLI exits non-zero on bad icons and zero on the shipped set', () => {
  const bad = spawnSync('node', ['scripts/ci/check-icons.mjs', dirWith(() => png(64, (x, y) => x < 6 && y < 6))], { encoding: 'utf8' });
  assert.equal(bad.status, 1);
  execFileSync('node', ['scripts/ci/check-icons.mjs']);
});
