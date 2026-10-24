import assert from 'node:assert/strict';
import { test } from 'node:test';
import { lint } from './commit-lint.mjs';

test('accepts Conventional Commits with optional scope and breaking mark', () => {
  for (const m of ['feat: add calculated fields', 'fix(ui): plural span count', 'refactor(core)!: typed errors\n\nBody text.', 'ci: pin actions']) {
    assert.deepEqual(lint(m), [], m);
  }
});

test('lets git-generated merge, revert and fixup commits pass', () => {
  for (const m of ['Merge branch main', 'Revert "feat: x"', 'fixup! feat: x']) assert.deepEqual(lint(m), []);
});

test('reports a bad type, an empty message, a long header and a missing blank line', () => {
  assert.match(lint('added stuff')[0], /does not start with/);
  assert.match(lint('feature: x')[0], /does not start with/);
  assert.match(lint('feat:x')[0], /does not start with/);
  assert.deepEqual(lint('# only a comment\n'), ['the message is empty']);
  assert.match(lint(`feat: ${'x'.repeat(220)}`).join(), /at most 200/);
  assert.deepEqual(lint('feat: x\nbody'), ['the second line must be empty']);
  assert.deepEqual(lint('fix: windows line endings\r\n\r\nbody'), []);
});

test('rejects co-author trailers and AI attribution anywhere in the message', () => {
  const trailer = 'Co-Authored-By: Someone <someone@example.com>';
  assert.match(lint(`feat: x\n\nbody\n\n${trailer}`).join(), /Co-Authored-By/);
  assert.match(lint(`feat: x\n\nbody\n\nco-authored-by: a <a@b.c>`).join(), /Co-Authored-By/);
  assert.match(lint('fix: x\n\nGenerated with Claude Code').join(), /AI "generated/);
  assert.match(lint('fix: x\n\n\u{1F916} done').join(), /robot/);
  assert.match(lint('fix: x\n\nSigned: Claude <noreply@anthropic.com>').join(), /AI author/);
  // Mentioning the word in prose is fine.
  assert.deepEqual(lint('docs: explain why the co-authored trailer is banned'), []);
});
