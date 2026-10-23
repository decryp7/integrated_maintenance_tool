import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { listLeaks, main, newLeaks } from './check-temp-leaks.mjs';

const quiet = () => {
  const out = [];
  return { out, log: { log: (m) => out.push(m), error: (m) => out.push(m) } };
};

function scratch() {
  const dir = mkdtempSync(join(tmpdir(), 'leakcheck-'));
  return { dir, done: () => rmSync(dir, { recursive: true, force: true }) };
}

test('lists only debrief-* and imt-* entries, sorted', () => {
  const s = scratch();
  try {
    for (const n of ['debrief-b', 'imt-a', 'other', 'debriefx', 'xdebrief-1']) mkdirSync(join(s.dir, n));
    assert.deepEqual(listLeaks(s.dir), ['debrief-b', 'imt-a']);
    assert.deepEqual(listLeaks(join(s.dir, 'missing')), []);
  } finally {
    s.done();
  }
});

test('newLeaks ignores what the baseline already had', () => {
  assert.deepEqual(newLeaks(['debrief-a', 'debrief-b'], ['debrief-a']), ['debrief-b']);
  assert.deepEqual(newLeaks(['debrief-a']), ['debrief-a']);
});

test('snapshot then check passes when nothing new appeared and fails naming the new folder', () => {
  const s = scratch();
  try {
    mkdirSync(join(s.dir, 'debrief-old'));
    const file = join(s.dir, 'base.json');
    const a = quiet();
    assert.equal(main(['--dir', s.dir, '--snapshot', file], a.log), 0);
    assert.deepEqual(JSON.parse(readFileSync(file, 'utf8')), ['debrief-old']);
    assert.equal(main(['--dir', s.dir, '--check', file], quiet().log), 0);
    mkdirSync(join(s.dir, 'debrief-find-1'));
    const b = quiet();
    assert.equal(main(['--dir', s.dir, '--check', file], b.log), 1);
    assert.match(b.out.join('\n'), /debrief-find-1/);
    assert.doesNotMatch(b.out.join('\n'), /debrief-old/);
  } finally {
    s.done();
  }
});

test('without a baseline every matching folder counts; a corrupt baseline counts as empty', () => {
  const s = scratch();
  try {
    mkdirSync(join(s.dir, 'imt-x'));
    assert.equal(main(['--dir', s.dir], quiet().log), 1);
    const bad = join(s.dir, 'bad.json');
    writeFileSync(bad, 'not json');
    assert.equal(main(['--dir', s.dir, '--check', bad], quiet().log), 1);
  } finally {
    s.done();
  }
});
