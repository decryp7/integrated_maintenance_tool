// Runs macos-smoke.sh against a stub .app (shell scripts, no window): a well-behaved one passes, one that does not quit
// in time, one that dies at start and an unknown sidecar fail. macOS only (PlistBuddy); skipped elsewhere.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT = fileURLToPath(new URL('./macos-smoke.sh', import.meta.url));
const mac = process.platform === 'darwin';

function stubApp(root, { main, extra = {} }) {
  const app = join(root, 'Debrief.app');
  const macos = join(app, 'Contents', 'MacOS');
  mkdirSync(macos, { recursive: true });
  writeFileSync(
    join(app, 'Contents', 'Info.plist'),
    `<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>debrief-app</string><key>CFBundleIdentifier</key><string>test.debrief.stub</string></dict></plist>`,
  );
  const put = (name, body) => {
    writeFileSync(join(macos, name), `#!/bin/bash\n${body}\n`);
    chmodSync(join(macos, name), 0o755);
  };
  put('debrief-app', main);
  put('debrief-store-worker', 'echo self-test ok');
  put('debrief-plugins', 'echo "[]"');
  put('debrief-ai-worker', 'echo "{}"');
  for (const [n, b] of Object.entries(extra)) put(n, b);
  return app;
}

function run(app) {
  return spawnSync('bash', [SCRIPT, app], { encoding: 'utf8', env: { ...process.env, SMOKE_QUIT: 'term', SETTLE: '1', QUIT_LIMIT: '2' } });
}

const withApp = (opts, fn) => {
  const root = mkdtempSync(join(tmpdir(), 'debrief-smoke-test-'));
  try {
    fn(run(stubApp(root, opts)));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
};

test('an app that stays up and quits on request passes', { skip: !mac }, () => {
  withApp({ main: "trap 'exit 0' TERM; while true; do sleep 0.1; done" }, (r) => {
    assert.equal(r.status, 0, r.stdout + r.stderr);
    assert.match(r.stdout, /macOS smoke OK/);
  });
});

test('an app that ignores the quit request fails after the limit', { skip: !mac }, () => {
  withApp({ main: "trap '' TERM; while true; do sleep 0.1; done" }, (r) => {
    assert.equal(r.status, 1);
    assert.match(r.stdout, /still running 2s after the quit request/);
  });
});

test('an app that dies at start fails', { skip: !mac }, () => {
  withApp({ main: 'exit 3' }, (r) => {
    assert.equal(r.status, 1);
    assert.match(r.stdout, /exited by itself/);
  });
});

test('a sidecar without a self-test entry fails', { skip: !mac }, () => {
  withApp({ main: 'sleep 5', extra: { 'debrief-new-thing': 'true' } }, (r) => {
    assert.equal(r.status, 1);
    assert.match(r.stdout, /debrief-new-thing has no self-test/);
  });
});

test('a failing sidecar fails the smoke', { skip: !mac }, () => {
  withApp({ main: 'sleep 5', extra: { 'debrief-store-worker': 'echo boom; exit 134' } }, (r) => {
    assert.equal(r.status, 1);
    assert.match(r.stdout, /failed with exit code 134/);
  });
});
