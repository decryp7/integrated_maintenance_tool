// The start-up benchmark's parsing and budget check, and one run against a stub app (a shell script that logs a
// "Startup timings" line and waits to be stopped). The stub needs a POSIX shell; skipped on Windows.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { chmodSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { evaluate, MARKS, median, parseTimingsLine, runOnce } from './startup-bench.mjs';

const fields = { WindowCreatedMs: 100, SetupDoneMs: 300, PageLoadMs: 600, FirstPaintMs: 700, WindowReadyMs: 710, WorkspaceLoadedMs: 720, FirstInteractiveMs: 730 };
const line = (extra = {}) => JSON.stringify({ '@t': '2026-10-24T00:00:00Z', '@m': 'Startup timings: …', Steps: 'app state=20', ...fields, ...extra });

test('reads the marks of the "Startup timings" line and ignores other lines', () => {
  const t = parseTimingsLine(line());
  assert.equal(t.marks.firstInteractive, 730);
  assert.equal(t.marks.windowCreated, 100);
  assert.equal(t.steps, 'app state=20');
  assert.equal(parseTimingsLine('{"@m":"Window main ready"}'), null);
  assert.equal(parseTimingsLine('not json'), null);
  assert.equal(parseTimingsLine(line({ FirstPaintMs: 'soon' })), null, 'a line without every mark is not counted');
  assert.equal(MARKS.length, 7);
});

test('medians per mark and the marks over budget', () => {
  assert.equal(median([3, 1, 2]), 2);
  assert.equal(median([4, 1, 2, 3]), 2.5);
  const runs = [700, 800, 900].map((ms) => ({ marks: { ...parseTimingsLine(line()).marks, firstInteractive: ms } }));
  const { medians, over } = evaluate(runs, { firstInteractive: 750, setupDone: 400 });
  assert.equal(medians.firstInteractive, 800);
  assert.deepEqual(over, [{ mark: 'firstInteractive', median: 800, budget: 750 }]);
  assert.deepEqual(evaluate(runs, undefined).over, []);
});

test('runs a start, reads its line and stops it', { skip: process.platform === 'win32' }, async () => {
  const dir = mkdtempSync(join(tmpdir(), 'debrief-bench-stub-'));
  try {
    const exe = join(dir, 'debrief-app');
    writeFileSync(exe, `#!/bin/sh\necho 'starting'\necho '${line()}'\nexec sleep 30\n`);
    chmodSync(exe, 0o755);
    const t = await runOnce(exe, dir, 10_000);
    assert.equal(t.marks.setupDone, 300);
    const quits = join(dir, 'quits');
    writeFileSync(quits, '#!/bin/sh\necho no timings\nexit 3\n');
    chmodSync(quits, 0o755);
    await assert.rejects(runOnce(quits, dir, 10_000), /exited \(code 3\)/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
