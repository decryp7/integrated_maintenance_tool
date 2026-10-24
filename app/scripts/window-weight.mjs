#!/usr/bin/env node
// Measures the JS a window loads before first paint, from the production build's esbuild metafile
// (`npx ng build --stats-json` writes dist/app/browser-stats.json). Paddock rule 35.
//
//   node scripts/window-weight.mjs main              # the main window: main -> bootstrap -> app + app.config
//   node scripts/window-weight.mjs popout            # a pop-out panel window: main -> bootstrap -> pop-out shell + app.config
//   node scripts/window-weight.mjs about             # a dialog window (`src/app/<name>/<name>-dialog.ts`)
//   node scripts/window-weight.mjs main --max 1100   # also fail when the total exceeds 1100 kB
//
// The walk follows static imports from the window's entry chunks (dynamic imports are the lazy chunks the window
// opens on demand and are not counted). It prints the heaviest chunks and the heaviest source packages (the
// directory under src/app, or the npm package). `npm run check:dialog-weight` and CI call it.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const base = (p) => p.replace(/^.*\//, '');

/** The entry sources whose chunks a window loads; a dialog's content is `src/app/<name>/<name>-dialog.ts`. */
export function windowEntries(kind) {
  const common = ['src/main.ts', 'src/bootstrap.ts'];
  if (kind === 'main') return [...common, 'src/app/app.ts', 'src/app/app.config.ts'];
  if (kind === 'popout') return [...common, 'src/app/popout/popout-shell.ts', 'src/app/app.config.ts'];
  return [
    ...common,
    'src/app/dialog-windows/dialog-shell.ts',
    'src/app/dialog-windows/dialog-bootstrap.ts',
    `src/app/${kind}/${kind}-dialog.ts`,
  ];
}

/** Indexes the metafile's JS outputs by file name. */
export function indexOutputs(outputs) {
  return new Map(Object.entries(outputs).filter(([f]) => f.endsWith('.js')).map(([f, o]) => [base(f), o]));
}

/** The output file whose first-class entry is the source file (a lazy chunk named after its entry point). */
export function chunkOf(byFile, source) {
  return [...byFile].find(([, o]) => o.entryPoint === source)?.[0];
}

/** Static closure of output files reachable from the roots (dynamic imports excluded). */
export function staticClosure(byFile, roots) {
  const seen = new Set();
  const stack = [...roots];
  while (stack.length) {
    const f = stack.pop();
    if (!f || seen.has(f) || !byFile.has(f)) continue;
    seen.add(f);
    for (const imp of byFile.get(f).imports ?? []) if (imp.kind === 'import-statement') stack.push(base(imp.path));
  }
  return seen;
}

/** The package a source file belongs to: the npm package, or the directory under src/app. */
export function packageOf(input) {
  const nm = input.match(/node_modules\/((?:@[^/]+\/)?[^/]+)/);
  if (nm) return nm[1];
  const app = input.match(/^src\/app\/([^/]+)/);
  if (app) return `src/app/${app[1]}`;
  return input.startsWith('src/') ? 'src' : input.split('/').slice(0, 2).join('/');
}

/** Measures one window kind: total bytes, chunk rows (heaviest first) and package rows (heaviest first). */
export function measure(outputs, kind) {
  const byFile = indexOutputs(outputs);
  const roots = windowEntries(kind).map((s) => chunkOf(byFile, s)).filter(Boolean);
  if (roots.length < 4) return null;
  const loaded = staticClosure(byFile, roots);
  const pkgs = new Map();
  let total = 0;
  const chunks = [];
  for (const f of loaded) {
    const o = byFile.get(f);
    total += o.bytes;
    const inputs = Object.entries(o.inputs ?? {}).sort((a, b) => b[1].bytesInOutput - a[1].bytesInOutput);
    chunks.push({ file: f, bytes: o.bytes, src: inputs[0]?.[0] });
    for (const [src, i] of inputs) pkgs.set(packageOf(src), (pkgs.get(packageOf(src)) ?? 0) + i.bytesInOutput);
  }
  chunks.sort((a, b) => b.bytes - a.bytes);
  const packages = [...pkgs].map(([name, bytes]) => ({ name, bytes })).sort((a, b) => b.bytes - a.bytes);
  return { kind, total, count: loaded.size, chunks, packages, files: loaded };
}

function main() {
  const args = process.argv.slice(2);
  const kind = args.find((a, i) => !a.startsWith('--') && args[i - 1] !== '--max') ?? 'main';
  const maxIdx = args.indexOf('--max');
  const maxKb = maxIdx >= 0 ? Number(args[maxIdx + 1]) : null;
  const statsPath = resolve(process.env.STATS_JSON ?? 'dist/app/browser-stats.json');
  const { outputs } = JSON.parse(readFileSync(statsPath, 'utf8'));
  const m = measure(outputs, kind);
  if (!m) {
    console.error(`could not find the "${kind}" window's entry chunks in ${statsPath}; is the build current?`);
    process.exit(2);
  }
  console.log(`JS the "${kind}" window loads before first paint (static closure of its entry chunks): ${(m.total / 1024).toFixed(1)} kB in ${m.count} chunks`);
  for (const r of m.chunks.slice(0, 15)) console.log(`  ${(r.bytes / 1024).toFixed(1).padStart(8)} kB  ${r.file}  (${r.src ?? ''})`);
  console.log('  heaviest packages:');
  for (const p of m.packages.slice(0, 15)) console.log(`  ${(p.bytes / 1024).toFixed(1).padStart(8)} kB  ${p.name}`);
  if (maxKb !== null && m.total / 1024 > maxKb) {
    console.error(`FAIL: ${(m.total / 1024).toFixed(1)} kB exceeds the ${maxKb} kB budget for the "${kind}" window`);
    process.exit(1);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
