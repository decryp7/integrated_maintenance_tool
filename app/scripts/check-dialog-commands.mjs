#!/usr/bin/env node
// The dialog command contract (ADR 0054). Every dialog declares, in `dialog-registry.ts`, the IPC commands its
// component may call (`commands`). Rust's guard allows a dialog window exactly those (plus the base commands), so a
// list that is too short is a dialog that silently does nothing, and one that is too long is a hole. This derives the
// real set by a type-checked call-graph walk (`dialog-commands-analysis.mjs`) and fails when
//   1. a dialog reaches a command it does not declare,
//   2. a dialog declares a command nothing reaches,
//   3. a command is not in the Rust command table, or the table does not let dialogs call it,
//   4. the shell's own base providers reach a command the base lists do not name.
// A reviewed exception (a command reached statically but never called from the dialog's window, or the reverse) is
// listed in `EXCEPTIONS` with the reason.
//
// Usage: node scripts/check-dialog-commands.mjs [--json]   (--json prints the derived lists, for regenerating the registry)
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import { createAnalysis, readRecordOfStringArrays, readRegistry, readStringArray } from './dialog-commands-analysis.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const APP = join(root, 'src/app');

/** Services whose methods are never called from a dialog window: the walk does not enter them (their commands are MAIN-only). */
export const HOST_ONLY_SERVICES = [
  // The dock and the pop-out service exist in a dialog's injector graph but have nothing to dock or pop out there.
  ['dock/dock.service.ts', 'DockService'],
  ['popout/popout.service.ts', 'PopoutService'],
  // The session belongs to the host window; nothing starts or saves it in a dialog window.
  ['lifecycle/session.service.ts', 'SessionService'],
  // A dialog window hands these two restarts to its host (`HOST_COMMAND_FORWARD`), because the session is saved there.
  ['lifecycle/lifecycle.service.ts', 'LifecycleService', 'restartElevated'],
  ['lifecycle/lifecycle.service.ts', 'LifecycleService', 'restartKeepingSession'],
];

/** Reviewed: `dialog id` -> { reached: commands reached statically but never called from the window, declared: declared on purpose without being reached } (the reason is the comment). */
export const EXCEPTIONS = {
  // Settings embeds every section's panels; some of their commands are only used when the panel is opened from its own pane.
};

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

const covers = (declared, command) => declared.some((d) => (d.endsWith('*') ? command.startsWith(d.slice(0, -1)) : d === command));

/** The problems of one dialog: derived `reached` against `declared`, `base` and the `table`. */
export function auditDialog({ id, declared, reached, base, table, exceptions = {} }) {
  const problems = [];
  const skip = exceptions.reached ?? [];
  const extraOk = exceptions.declared ?? [];
  const need = [...reached.keys()].filter((c) => !covers(base, c) && !skip.includes(c));
  for (const c of need) {
    if (!table.has(c)) problems.push(`${id}: calls ${c} (${reached.get(c)}), which is not in the command table`);
    else if (!covers(declared, c)) problems.push(`${id}: calls ${c} (${reached.get(c)}) but does not declare it in 'commands'`);
  }
  for (const d of declared) {
    const matches = d.endsWith('*') ? [...reached.keys()].some((c) => c.startsWith(d.slice(0, -1))) : reached.has(d);
    if (!matches && !extraOk.includes(d)) problems.push(`${id}: declares ${d} but nothing it reaches calls it`);
    if (!d.endsWith('*')) {
      const classes = table.get(d);
      if (!classes) problems.push(`${id}: declares ${d}, which is not in the command table`);
      else if (!classes.includes('DIALOG')) problems.push(`${id}: declares ${d}, which the command table does not let a dialog call (add DIALOG to its entry, reviewed)`);
    }
  }
  return problems;
}

export function run(rootDir = root) {
  const analysis = createAnalysis(join(rootDir, 'tsconfig.app.json'));
  const app = join(rootDir, 'src/app');
  const table = parseTable(readFileSync(join(rootDir, 'src-tauri/src/lib.rs'), 'utf8'));
  const known = new Set(table.keys());
  const registryFile = join(app, 'dialog-windows/dialog-registry.ts');
  const kitBase = readStringArray(analysis, join(app, 'dialog-windows/kit/dialog-kind.ts'), 'KIT_BASE_COMMANDS') ?? [];
  const appBase = readStringArray(analysis, join(app, 'dialog-windows/dialog-def.ts'), 'DIALOG_BASE_COMMANDS') ?? [];
  const syncBase0 = readStringArray(analysis, join(app, 'dialog-windows/dialog-def.ts'), 'DIALOG_SYNC_COMMANDS') ?? [];
  const bootBase = readStringArray(analysis, join(app, 'dialog-windows/dialog-def.ts'), 'DIALOG_BOOT_COMMANDS') ?? [];
  const base = [...kitBase, ...appBase, ...bootBase, ...syncBase0];
  const groupCommands = readRecordOfStringArrays(analysis, join(app, 'dialog-windows/dialog-def.ts'), 'DIALOG_BACKEND_COMMANDS');
  const problems = [];
  const derived = {};

  // The shell and the base providers every dialog window boots.
  const find = (file, name) => {
    const sf = analysis.program.getSourceFile(join(app, file));
    let out = null;
    const visit = (n) => {
      if ((ts.isClassDeclaration(n) || ts.isFunctionDeclaration(n)) && n.name?.text === name) out = n;
      ts.forEachChild(n, visit);
    };
    if (sf) visit(sf);
    return out;
  };
  const shell = find('dialog-windows/dialog-shell.ts', 'DialogShell');
  const providers = find('dialog-windows/dialog-bootstrap.ts', 'dialogBaseProviders');
  if (!shell || !providers) throw new Error('the dialog shell or its base providers moved: update check-dialog-commands.mjs');
  // The UI state a dialog window syncs with its host is applied by `sharedUiState`; what applying it calls is listed by
  // hand in DIALOG_SYNC_COMMANDS (the analysis cannot tell which entries a given dialog receives).
  const syncFn = find('popout/shared-state.ts', 'sharedUiState');
  const boundary = new Set(syncFn ? [syncFn] : []);
  // Host-only services: the dock and the pop-out service exist in a dialog's injector graph (a dialog may inject the
  // perspectives or the command registry) but have nothing to dock or pop out there; their commands are MAIN-only.
  for (const [file, name, member] of HOST_ONLY_SERVICES) {
    const c = find(file, name);
    const target = member ? c?.members.find((m) => m.name?.getText?.() === member) : c;
    if (!target) problems.push(`HOST_ONLY_SERVICES names ${name}${member ? `.${member}` : ''} (${file}), which does not exist: update check-dialog-commands.mjs`);
    else boundary.add(target);
  }
  const baseReached = analysis.reach([...analysis.componentEntries(shell), providers], known, boundary).commands;
  for (const c of baseReached.keys()) {
    if (!covers(base, c)) problems.push(`the dialog shell reaches ${c} (${baseReached.get(c)}); add it to DIALOG_BASE_COMMANDS`);
  }
  for (const c of appBase) if (!baseReached.has(c)) problems.push(`DIALOG_BASE_COMMANDS lists ${c}, which the dialog shell and base providers never call`);

  for (const entry of readRegistry(analysis, registryFile)) {
    if (!entry.component) {
      problems.push(`${entry.id}: its load() does not name a component`);
      continue;
    }
    const cls = analysis.declsOf(entry.component).find(ts.isClassDeclaration);
    if (!cls) {
      problems.push(`${entry.id}: cannot find the component class`);
      continue;
    }
    const { commands, dynamic } = analysis.reach(analysis.componentEntries(cls), known, boundary);
    for (const where of dynamic) problems.push(`${entry.id}: a command is called by a computed name (${where}); call it by a literal so it can be declared`);
    // What the providers of the backend groups it names send by themselves is implied (see DIALOG_BACKEND_COMMANDS).
    const own = [...base, ...entry.backends.flatMap((b) => groupCommands[b] ?? [])];
    derived[entry.id] = [...commands.keys()].filter((c) => !covers(own, c)).sort();
    problems.push(...auditDialog({ id: entry.id, declared: entry.commands, reached: commands, base: own, table, exceptions: EXCEPTIONS[entry.id] }));
  }
  return { problems, derived, base, dialogs: Object.keys(derived).length, commands: table.size };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const r = run();
  if (process.argv.includes('--json')) {
    console.log(JSON.stringify(r.derived, null, 2));
  } else if (r.problems.length) {
    console.error(r.problems.join('\n'));
    process.exit(1);
  } else {
    console.log(`dialog commands: ${r.dialogs} dialogs, ${r.commands} commands in the table, every declared list matches what the dialogs reach`);
  }
}
