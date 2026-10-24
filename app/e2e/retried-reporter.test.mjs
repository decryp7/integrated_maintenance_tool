import { test } from 'node:test';
import assert from 'node:assert/strict';
import Reporter, { render } from './retried-reporter.mjs';

const t = (retry, status = 'passed') => [
  { parent: { project: () => ({ name: 'webkit' }) }, location: { file: '/x/app/e2e/home.e2e.ts' }, titlePath: () => ['', 'webkit', 'home.e2e.ts', 'Home', 'opens | fast'] },
  { retry, status },
];

test('only tests that passed after a retry are listed', () => {
  const r = new Reporter();
  r.onTestEnd(...t(0));
  r.onTestEnd(...t(1, 'failed'));
  r.onTestEnd(...t(2));
  assert.equal(r.flaky.length, 1);
  assert.deepEqual(r.flaky[0], { project: 'webkit', file: 'home.e2e.ts', title: 'Home > opens \\| fast', retries: 2 });
});

test('renders a table, or nothing when there is no flake', () => {
  assert.equal(render([]), '');
  assert.match(render(new Reporter().flaky.concat([{ project: 'p', file: 'f', title: 't', retries: 1 }])), /\| p \| f \| t \| 1 \|/);
});
