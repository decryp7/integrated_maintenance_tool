import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import ts from 'typescript';
import { auditDialog, parseTable } from './check-dialog-commands.mjs';
import { createAnalysis } from './dialog-commands-analysis.mjs';

test('reads the command table and which window classes may call each command', () => {
  const t = parseTable(`
    Query [WINDOW] => workspace_commands::workspace_summary,
    Mutation [MAIN] => window_manager::resolve_close,
    Quiet [MAIN | DIALOG] limit 4 * MIB => a::b::both,`);
  assert.deepEqual(t.get('workspace_summary'), ['MAIN', 'POPOUT', 'DIALOG']);
  assert.deepEqual(t.get('resolve_close'), ['MAIN']);
  assert.deepEqual(t.get('both'), ['MAIN', 'DIALOG']);
});

const table = parseTable('Query [WINDOW] => a::read_it,\nMutation [WINDOW] => a::write_it,\nMutation [MAIN] => a::main_only,\nQuiet [WINDOW] => a::base_one,');
const reached = (...names) => new Map(names.map((n) => [n, 'x.ts:1']));

test('fails a dialog that reaches a command it does not declare, and one that declares a command nothing reaches', () => {
  const run = (declared, r, exceptions) => auditDialog({ id: 'd', declared, reached: reached(...r), base: ['base_one'], table, exceptions });
  assert.deepEqual(run(['read_it'], ['read_it', 'base_one']), []);
  assert.match(run([], ['read_it'])[0], /calls read_it .* does not declare it/);
  assert.match(run(['read_it', 'write_it'], ['read_it'])[0], /declares write_it but nothing it reaches calls it/);
  // A reviewed exception in either direction.
  assert.deepEqual(run(['read_it'], ['read_it', 'write_it'], { reached: ['write_it'] }), []);
  assert.deepEqual(run(['read_it', 'write_it'], ['read_it'], { declared: ['write_it'] }), []);
});

test('fails a command outside the table and one the table does not open to dialogs (the resolve_close class)', () => {
  const problems = auditDialog({ id: 'd', declared: ['main_only', 'nope'], reached: reached('main_only', 'nope'), base: [], table });
  assert.ok(problems.some((p) => /declares main_only, which the command table does not let a dialog call/.test(p)), problems.join('\n'));
  assert.ok(problems.some((p) => /nope.* not in the command table/.test(p)), problems.join('\n'));
});

test('a trailing * declares a prefix, which must match something', () => {
  const t = parseTable('Query [WINDOW] => a::ui_sync,\nQuery [WINDOW] => a::ui_theme,');
  assert.deepEqual(auditDialog({ id: 'd', declared: ['ui_*'], reached: reached('ui_sync', 'ui_theme'), base: [], table: t }), []);
  assert.match(auditDialog({ id: 'd', declared: ['zz_*'], reached: reached('ui_sync'), base: [], table: t }).join(), /declares zz_\*/);
});

/** A small TypeScript program to walk: abstract backends with implementations, an object-literal factory, a template. */
function fixture(files) {
  const dir = mkdtempSync(join(tmpdir(), 'dlgcmd-'));
  mkdirSync(join(dir, 'src'));
  writeFileSync(join(dir, 'tsconfig.json'), JSON.stringify({ compilerOptions: { target: 'ES2022', module: 'ESNext', moduleResolution: 'bundler', strict: true, experimentalDecorators: true, noEmit: true }, include: ['src/**/*.ts'] }));
  writeFileSync(join(dir, 'src/prelude.ts'), 'export declare function inject<T>(t: abstract new (...a: any[]) => T): T;\nexport declare function Component(o: { template?: string; imports?: unknown[] }): ClassDecorator;');
  for (const [name, text] of Object.entries(files)) writeFileSync(join(dir, 'src', name), text);
  const a = createAnalysis(join(dir, 'tsconfig.json'));
  const cls = (file, name) => {
    let out = null;
    const visit = (n) => {
      if (ts.isClassDeclaration(n) && n.name?.text === name) out = n;
      ts.forEachChild(n, visit);
    };
    visit(a.program.getSourceFile(join(dir, 'src', file)));
    return out;
  };
  const known = new Set(['read_it', 'write_it', 'main_only', 'deep_one', 'other_one']);
  return { a, cls, known, reach: (file, name, boundary) => [...a.reach(a.componentEntries(cls(file, name)), known, boundary).commands.keys()].sort() };
}

test('follows an injected abstract backend to its implementations, and only the methods the component uses', () => {
  const f = fixture({
    'backend.ts': "export abstract class Backend { abstract read(): void; abstract write(): void; }\nexport class Real extends Backend { read() { this.call('read_it'); } write() { this.call('write_it'); } call(c: string) { return c; } }",
    'dialog.ts': "import { inject, Component } from './prelude';\nimport { Backend } from './backend';\n@Component({ template: '' })\nexport class D { private readonly b = inject(Backend); go() { this.b.read(); } }",
  });
  assert.deepEqual(f.reach('dialog.ts', 'D'), ['read_it']);
});

test('follows a method named only in the template, through the member it is called on', () => {
  const f = fixture({
    'store.ts': "export class Store { save() { this.call('write_it'); } load() { this.call('read_it'); } call(c: string) { return c; } }",
    'dialog.ts': "import { inject, Component } from './prelude';\nimport { Store } from './store';\n@Component({ template: '<button (click)=\"store.save()\">Save</button>' })\nexport class D { protected readonly store = inject(Store); }",
  });
  assert.deepEqual(f.reach('dialog.ts', 'D'), ['write_it']);
});

test('follows a factory that returns an object literal as an interface, by the members used', () => {
  const f = fixture({
    'io.ts': "export interface Io { open(): void; close(): void; }\nexport function makeIo(): Io { return { open: () => call('write_it'), close: () => call('read_it') }; }\ndeclare function call(c: string): void;",
    'dialog.ts': "import { Component } from './prelude';\nimport { Io, makeIo } from './io';\n@Component({ template: '' })\nexport class D { private readonly io: Io = makeIo(); go() { this.io.close(); } }",
  });
  assert.deepEqual(f.reach('dialog.ts', 'D'), ['read_it']);
});

test('does not enter a boundary (a host-only service, the synced UI state)', () => {
  const f = fixture({
    'host.ts': "export class Host { act() { this.call('main_only'); } call(c: string) { return c; } }",
    'dialog.ts': "import { inject, Component } from './prelude';\nimport { Host } from './host';\n@Component({ template: '' })\nexport class D { private readonly h = inject(Host); go() { this.h.act(); } }",
  });
  assert.deepEqual(f.reach('dialog.ts', 'D'), ['main_only']);
  assert.deepEqual(f.reach('dialog.ts', 'D', new Set([f.cls('host.ts', 'Host')])), []);
});

test('reports a command called by a computed name, but not a wrapper that passes its parameter on', () => {
  const f = fixture({
    'dialog.ts': "import { Component } from './prelude';\nfunction call(c: string) { return c; }\nfunction wrap(cmd: string) { return call(cmd); }\n@Component({ template: '' })\nexport class D { name = String(Math.random()); go() { call(this.name); wrap('deep_one'); } }",
  });
  const r = f.a.reach(f.a.componentEntries(f.cls('dialog.ts', 'D')), f.known);
  assert.deepEqual([...r.commands.keys()], ['deep_one']);
  assert.equal(r.dynamic.length, 1);
});
