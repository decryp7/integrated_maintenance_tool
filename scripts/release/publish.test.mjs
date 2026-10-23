import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { filesUnder, parseArgs, publish } from './publish.mjs';

const args = ['--api', 'https://h/api/v1/', '--repo', 'o/r', '--tag', 'v1.2.0', '--notes', 'n.md', '--dir', 'a'];

test('arguments are parsed and checked', () => {
  assert.deepEqual(parseArgs(args), { api: 'https://h/api/v1', repo: 'o/r', tag: 'v1.2.0', notes: 'n.md', dir: 'a' });
  assert.throws(() => parseArgs(args.slice(0, 8)), /--dir is required/);
  assert.throws(() => parseArgs([...args.slice(0, 4), '--tag', 'latest', ...args.slice(6)]), /vX.Y.Z/);
  assert.throws(() => parseArgs(['--bogus', 'x']), /unknown/);
});

function fixture() {
  const dir = mkdtempSync(join(tmpdir(), 'publish-'));
  mkdirSync(join(dir, 'a', 'nsis'), { recursive: true });
  writeFileSync(join(dir, 'a', 'latest.json'), '{}');
  writeFileSync(join(dir, 'a', 'nsis', 'App_1.2.0_x64-setup.exe'), 'exe');
  writeFileSync(join(dir, 'n.md'), 'notes');
  return dir;
}

test('files are found recursively in a stable order', () => {
  const dir = fixture();
  assert.deepEqual(filesUnder(join(dir, 'a')).map((f) => f.slice(dir.length)), ['/a/latest.json', '/a/nsis/App_1.2.0_x64-setup.exe']);
});

function fakeApi(existing) {
  const calls = [];
  const fetchImpl = async (url, init) => {
    calls.push(`${init.method} ${url}`);
    const json = (status, body) => ({ status, ok: status < 300, json: async () => body, text: async () => JSON.stringify(body) });
    if (init.method === 'GET') return existing ? json(200, existing) : json(404, {});
    if (url.endsWith('/releases')) return json(201, { id: 7, html_url: 'https://h/o/r/releases/tag/v1.2.0', assets: [] });
    return json(201, {});
  };
  return { calls, fetchImpl };
}

test('a new release is created and every file uploaded', async () => {
  const dir = fixture();
  const { calls, fetchImpl } = fakeApi(null);
  const url = await publish({ api: 'https://h/api/v1', repo: 'o/r', tag: 'v1.2.0', notes: join(dir, 'n.md'), dir: join(dir, 'a') }, 't', fetchImpl, () => {});
  assert.equal(url, 'https://h/o/r/releases/tag/v1.2.0');
  assert.deepEqual(calls, [
    'GET https://h/api/v1/repos/o/r/releases/tags/v1.2.0',
    'POST https://h/api/v1/repos/o/r/releases',
    'POST https://h/api/v1/repos/o/r/releases/7/assets?name=latest.json',
    'POST https://h/api/v1/repos/o/r/releases/7/assets?name=App_1.2.0_x64-setup.exe',
  ]);
});

test('a re-run reuses the release and skips files already attached', async () => {
  const dir = fixture();
  const { calls, fetchImpl } = fakeApi({ id: 9, html_url: 'u', assets: [{ name: 'latest.json' }] });
  await publish({ api: 'https://h/api/v1', repo: 'o/r', tag: 'v1.2.0', notes: join(dir, 'n.md'), dir: join(dir, 'a') }, 't', fetchImpl, () => {});
  assert.deepEqual(calls.slice(1), ['POST https://h/api/v1/repos/o/r/releases/9/assets?name=App_1.2.0_x64-setup.exe']);
});

test('an API error names the call, not the token', async () => {
  const dir = fixture();
  const fetchImpl = async (url, init) =>
    init.method === 'GET' ? { status: 404, ok: false, json: async () => ({}) } : { status: 403, ok: false, text: async () => 'forbidden' };
  await assert.rejects(
    publish({ api: 'https://h/api/v1', repo: 'o/r', tag: 'v1.2.0', notes: join(dir, 'n.md'), dir: join(dir, 'a') }, 'secret-token', fetchImpl, () => {}),
    (e) => /POST https:\/\/h\/api\/v1\/repos\/o\/r\/releases: HTTP 403 forbidden/.test(e.message) && !e.message.includes('secret-token'),
  );
});
