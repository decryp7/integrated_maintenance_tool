// Run: node --test scripts/build-info.test.mjs
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { buildInfo, defineArg, gitShortHead, ngArgs } from './build-info.mjs';

const now = () => new Date('2026-10-17T23:59:00Z');

test('uses the short commit and the UTC date', () => {
  assert.deepEqual(buildInfo({ head: () => 'abc1234', now }), { commit: 'abc1234', date: '2026-10-17' });
});

test('falls back to unknown when git is unavailable or answers nonsense', () => {
  assert.equal(buildInfo({ head: () => null, now }).commit, 'unknown');
  assert.equal(buildInfo({ head: () => 'fatal: not a git repository', now }).commit, 'unknown');
});

test('never throws when git cannot run', () => {
  assert.equal(gitShortHead('/definitely/not/a/dir'), null);
});

test('builds the define argument and appends it to the ng arguments', () => {
  const info = { commit: 'abc1234', date: '2026-10-17' };
  assert.equal(defineArg(info), 'DEBRIEF_BUILD_INFO={"commit":"abc1234","date":"2026-10-17"}');
  assert.deepEqual(ngArgs(['build', '--configuration', 'production'], info), ['build', '--configuration', 'production', '--define', defineArg(info)]);
});
