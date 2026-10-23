import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { problems } from './check-quarantine.mjs';

const table = (d) => `| Test | Reason | Environment | Expires |\n| --- | --- | --- | --- |\n| \`a.e2e.ts\` | why | where | ${d} |\n`;

test('a current date passes', () => assert.deepEqual(problems(table('2027-01-31'), '2026-10-24'), []));
test('an expired date fails', () => assert.match(problems(table('2026-10-23'), '2026-10-24')[0], /expired on 2026-10-23/));
test('today is still current', () => assert.deepEqual(problems(table('2026-10-24'), '2026-10-24'), []));
test('a missing, malformed or impossible date fails', () => {
  for (const d of ['', 'soon', '2027-02-30', '31/12/2026']) assert.match(problems(table(d), '2026-10-24')[0], /must be a date/);
});
test('a date more than six months out fails', () => assert.match(problems(table('2028-01-01'), '2026-10-24')[0], /6 months/));
test('a table without the column or without a table fails', () => {
  assert.match(problems('| Test | Reason |\n| --- | --- |\n| a | b |\n', '2026-10-24')[0], /no "Expires" column/);
  assert.match(problems('nothing', '2026-10-24')[0], /no quarantine table/);
});
test('the real quarantine file is valid today', () => {
  const md = readFileSync(new URL('../../docs/testing/quarantine.md', import.meta.url), 'utf8');
  assert.deepEqual(problems(md, '2026-10-24'), []);
});
