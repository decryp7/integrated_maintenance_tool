import assert from 'node:assert/strict';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { main, violations } from './check-smoke-dialogs.mjs';

test('flags the in-page dialog helper and role=dialog selectors, not comments', () => {
  assert.equal(violations("const d = inPageDialog('Settings');").length, 1);
  assert.equal(violations(`$("[role='dialog']")`).length, 1);
  assert.equal(violations('// inPageDialog is for plain confirms').length, 0);
  assert.equal(violations("await withDialogWindow('settings', async () => {});").length, 0);
});

test('the real smoke specs are clean', () => {
  const specs = join(dirname(fileURLToPath(import.meta.url)), '../../app/e2e-tauri/specs');
  const out = [];
  assert.equal(main(specs, { error: (m) => out.push(m) }), 0, out.join('\n'));
});
