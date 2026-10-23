#!/usr/bin/env node
// A dialog window is its own webview of the DIALOG window class, which may call only the commands the table marks for
// it (`src-tauri/src/lib.rs`, `command_table!`). A dialog whose component (or a service, store or backend it injects)
// calls a command outside that set fails at runtime with "not available in this window" and does nothing. This reads
// the table, follows each registered dialog's `inject(...)` edges through relative imports, collects the commands the
// reached files call, and reports every denied one. The fix is to return the dialog's result to its opener (which
// runs the command) or to mark the command for DIALOG deliberately; a known and reviewed exception is listed in
// `EXCEPTIONS` with the reason.
//
// Usage: node scripts/check-dialog-commands.mjs
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const APP = join(root, 'src/app');

/** Reviewed: `dialog id` -> commands it reaches by injection but never calls from its window (the reason). */
export const EXCEPTIONS = {};

/** command name -> the window classes (`MAIN`, `POPOUT`, `DIALOG`, `WINDOW`) that may call it. */
export function parseTable(rs) {
  const out = new Map();
  for (const m of rs.matchAll(/^\s*(?:Query|Mutation|Quiet)\s*\[([^\]]+)\][^=]*=>\s*([\w:]+)\s*,/gm)) {
    const name = m[2].split('::').pop();
    const classes = m[1].split('|').map((c) => c.trim());
    out.set(name, classes.includes('WINDOW') ? ['MAIN', 'POPOUT', 'DIALOG'] : classes);
  }
  return out;
}

/** dialog id -> component file, from the registry's lazy imports. */
export function parseRegistry(ts, dir) {
  const out = [];
  for (const m of ts.matchAll(/\{ id: '([^']+)'[^\n]*?import\('([^']+)'\)/g)) out.push([m[1], resolveTs(dir, m[2])]);
  return out;
}

function resolveTs(dir, spec) {
  const base = resolve(dir, spec);
  return [base + '.ts', join(base, 'index.ts')].find(existsSync) ?? null;
}

const read = (f) => readFileSync(f, 'utf8');
const strip = (src) => src.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');

/** The commands a file calls itself: `this.call('x')`, `invoke('x')`, `this.invoke('x')`. */
export function callsIn(src) {
  const out = new Set();
  for (const m of strip(src).matchAll(/\b(?:call|invoke)(?:<[^>(]*>)?\(\s*'([a-z][a-z0-9_]*)'/g)) out.add(m[1]);
  return out;
}

/** The injected names of a file: `name = inject(Class)` / `name: T = inject(Class)` -> the file of Class (a relative import). */
export function injections(src, file) {
  const code = strip(src);
  const imports = new Map();
  for (const m of code.matchAll(/import\s*\{([^}]*)\}\s*from\s*'(\.[^']*)'/g)) {
    for (const n of m[1].split(',')) imports.set(n.trim().split(/\s+as\s+/).pop(), m[2]);
  }
  const out = new Map();
  for (const m of code.matchAll(/([A-Za-z_$][\w$]*)\s*(?::[^=;()]+)?=\s*inject(?:<[^>]*>)?\(\s*([A-Za-z_$][\w$]*)/g)) {
    const spec = imports.get(m[2]);
    const f = spec && resolveTs(dirname(file), spec);
    if (f) out.set(m[1], f);
  }
  return out;
}

/** The body of method `name` in `code` (all of them, when several classes in the file define it), by brace matching. */
export function methodBodies(code, name) {
  const out = [];
  const re = new RegExp(`(?:^|[\\s.])(?:async\\s+)?${name}\\s*(?:<[^>(]*>)?\\([^)]*\\)\\s*(?::[^{;=]+)?\\{`, 'gm');
  for (const m of code.matchAll(re)) {
    let i = m.index + m[0].length;
    let depth = 1;
    const start = i;
    while (i < code.length && depth > 0) depth += code[i] === '{' ? 1 : code[i] === '}' ? -1 : 0, i++;
    out.push(code.slice(start, i));
  }
  return out;
}

/** The methods of the injected `name` that `code` calls (`name.m(` / `this.name.m(`). */
function usedMethods(code, name) {
  const used = new Set();
  for (const m of code.matchAll(new RegExp(`\\b${name.replace(/\$/g, '\\$')}\\s*\\.\\s*([A-Za-z_$][\\w$]*)\\s*[(<]`, 'g'))) used.add(m[1]);
  return used;
}

/** Every command the dialog can reach, with the file that calls it: its own file in full, injected classes by the methods used. */
export function reachable(entry) {
  const seen = new Set();
  const found = new Map();
  const visit = (file, code) => {
    for (const c of callsIn(code)) if (!found.has(c)) found.set(c, file);
    for (const [name, target] of injections(read(file), file)) {
      for (const method of usedMethods(code, name)) {
        const key = `${target}#${method}`;
        if (seen.has(key)) continue;
        seen.add(key);
        const tsrc = strip(read(target));
        for (const body of methodBodies(tsrc, method)) visit(target, body);
      }
    }
  };
  visit(entry, strip(read(entry)));
  return found;
}

export function audit(table, registry, exceptions = EXCEPTIONS) {
  const problems = [];
  for (const [id, file] of registry) {
    if (!file) continue;
    for (const [cmd, via] of reachable(file)) {
      const classes = table.get(cmd);
      if (!classes || classes.includes('DIALOG') || exceptions[id]?.includes(cmd)) continue;
      problems.push(`${id}: calls ${cmd} (${via.replace(root + '/', '')}), which a dialog window may not call`);
    }
  }
  return problems;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const table = parseTable(read(resolve(root, 'src-tauri/src/lib.rs')));
  const registry = parseRegistry(read(join(APP, 'dialog-windows/dialog-registry.ts')), join(APP, 'dialog-windows'));
  const problems = audit(table, registry);
  if (problems.length) {
    console.error(problems.join('\n'));
    process.exit(1);
  }
  console.log(`dialog commands: ${registry.length} dialogs, ${table.size} commands, no denied calls`);
}
