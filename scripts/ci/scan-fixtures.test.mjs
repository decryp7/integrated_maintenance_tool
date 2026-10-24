import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { isHarmlessIp, main, scanText } from './scan-fixtures.mjs';

const kinds = (t, allow) => scanText(t, allow).map((f) => `${f.kind}:${f.text}`);

test('documentation, private and loopback addresses pass; public ones are flagged', () => {
  for (const ip of [[192, 0, 2, 7], [198, 51, 100, 1], [203, 0, 113, 9], [10, 1, 2, 3], [172, 20, 0, 1], [192, 168, 1, 1], [127, 0, 0, 1], [255, 255, 255, 0]]) {
    assert.ok(isHarmlessIp(ip), ip.join('.'));
  }
  assert.deepEqual(kinds('client 52.14.9.77 connected'), ['public-ip:52.14.9.77']);
  assert.deepEqual(kinds('client 192.0.2.4 connected'), []);
});

test('version strings are not addresses', () => {
  assert.deepEqual(kinds('Windows version 52.14.9.77'), []);
  assert.deepEqual(kinds('agent v52.14.9.77'), []);
  assert.deepEqual(kinds('build 10.0.19045.1234 ok'), []);
});

test('emails with a reserved domain pass, real-looking ones are flagged', () => {
  assert.deepEqual(kinds('mail alice@example.com, bob@corp.example.test'), []);
  assert.deepEqual(kinds('mail alice@acme-bank.com'), ['email:alice@acme-bank.com']);
});

test('company-internal hostnames are flagged, namespaces are not', () => {
  assert.deepEqual(kinds('connect db01.hq.acme.corp:5432'), ['internal-host:db01.hq.acme.corp']);
  assert.deepEqual(kinds('at Microsoft.Extensions.Internal.Thing'), []);
});

test('the allow list suppresses a reviewed value', () => {
  assert.deepEqual(kinds('x 52.14.9.77', { values: ['52.14.9.77'] }), []);
});

test('main exits 1 on a finding and 0 on a clean folder', () => {
  const dir = mkdtempSync(join(tmpdir(), 'debrief-scan-test-'));
  const quiet = { log() {}, error() {} };
  try {
    mkdirSync(join(dir, 'a'));
    writeFileSync(join(dir, 'a', 'ok.log'), 'host.example.test 192.0.2.1 a@example.com\n');
    assert.equal(main([dir], quiet), 0);
    writeFileSync(join(dir, 'a', 'bad.log'), 'from jane.doe@acme-bank.com\n');
    assert.equal(main([dir], quiet), 1);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
