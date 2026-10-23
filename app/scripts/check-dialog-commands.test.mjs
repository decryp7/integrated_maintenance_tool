import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { audit, callsIn, methodBodies, parseTable, reachable } from './check-dialog-commands.mjs';

test('reads the command table and which window classes may call each command', () => {
  const t = parseTable(`
    Query [WINDOW] => workspace_commands::workspace_summary,
    Mutation [MAIN] => window_manager::resolve_close,
    Quiet [MAIN | DIALOG] limit 4 * MIB => a::b::both,`);
  assert.deepEqual(t.get('workspace_summary'), ['MAIN', 'POPOUT', 'DIALOG']);
  assert.deepEqual(t.get('resolve_close'), ['MAIN']);
  assert.deepEqual(t.get('both'), ['MAIN', 'DIALOG']);
});

test('finds the commands a source calls, ignoring comments', () => {
  assert.deepEqual([...callsIn("a(){ return this.call<void>('resolve_close', {}); }\n// this.call('nope')\n x = invoke('open_it')")], ['resolve_close', 'open_it']);
  assert.equal(methodBodies('class A { go(x: number) { return this.call(\'k\'); } stay() { return 1; } }', 'go').length, 1);
});

test('fails a dialog that reaches a main-only command through an injected backend, and only for the methods it uses', () => {
  const dir = mkdtempSync(join(tmpdir(), 'dlg-'));
  mkdirSync(join(dir, 'x'));
  writeFileSync(join(dir, 'x/backend.ts'), "export class Backend { close() { return this.call('resolve_close'); } peek() { return this.call('workspace_summary'); } }");
  const dialog = (method) =>
    writeFileSync(join(dir, 'x/dialog.ts'), `import { Backend } from './backend';\nexport class D { private readonly backend = inject(Backend); go() { return this.backend.${method}(); } }`);
  const table = parseTable('Mutation [MAIN] => w::resolve_close,\nQuery [WINDOW] => w::workspace_summary,');
  dialog('close');
  assert.equal(audit(table, [['d', join(dir, 'x/dialog.ts')]]).length, 1);
  assert.deepEqual([...reachable(join(dir, 'x/dialog.ts')).keys()], ['resolve_close']);
  dialog('peek');
  assert.deepEqual(audit(table, [['d', join(dir, 'x/dialog.ts')]]), []);
  dialog('close');
  assert.deepEqual(audit(table, [['d', join(dir, 'x/dialog.ts')]], { d: ['resolve_close'] }), []);
});
