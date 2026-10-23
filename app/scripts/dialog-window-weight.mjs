#!/usr/bin/env node
// Measures the JS a dialog window loads on first paint, from the production build's esbuild metafile
// (`npx ng build --stats-json` writes dist/app/browser-stats.json).
//
//   node scripts/dialog-window-weight.mjs about            # report: the chunks the window loads and their sizes
//   node scripts/dialog-window-weight.mjs about --max 480  # also fail when the total exceeds 480 kB (`npm run check:dialog-weight`)
//
// A dialog window boots `main` -> `bootstrap` -> its shell, its base providers and its content component. The walk
// follows static imports from those entries and, for the dialog's own content, its dynamic import; the lazy chunks
// a dialog opens on demand (the third-party notices text) are listed apart and not counted.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const args = process.argv.slice(2);
const dialog = args.find((a) => !a.startsWith('--')) ?? 'about';
const maxIdx = args.indexOf('--max');
const maxKb = maxIdx >= 0 ? Number(args[maxIdx + 1]) : null;
const statsPath = resolve(process.env.STATS_JSON ?? 'dist/app/browser-stats.json');
const { outputs } = JSON.parse(readFileSync(statsPath, 'utf8'));

const byFile = new Map(Object.entries(outputs).filter(([f]) => f.endsWith('.js')).map(([f, o]) => [f.replace(/^.*\//, ''), o]));
const base = (p) => p.replace(/^.*\//, '');
/** The output file whose first-class entry is the source file (a lazy chunk named after its entry point). */
function chunkOf(source) {
  return [...byFile].find(([, o]) => o.entryPoint === source)?.[0];
}

/** Static closure of output files reachable from the roots (dynamic imports excluded). */
function staticClosure(roots) {
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

const roots = [
  chunkOf('src/main.ts'),
  chunkOf('src/bootstrap.ts'),
  chunkOf('src/app/dialog-windows/dialog-shell.ts'),
  chunkOf('src/app/dialog-windows/dialog-bootstrap.ts'),
  chunkOf(`src/app/${dialog}/${dialog}-dialog.ts`),
].filter(Boolean);
if (roots.length < 4) {
  console.error(`could not find the dialog entry chunks in ${statsPath} (found ${roots.length}); is the build current?`);
  process.exit(2);
}
const loaded = staticClosure(roots);
const size = (f) => byFile.get(f).bytes;
const total = [...loaded].reduce((n, f) => n + size(f), 0);
const rows = [...loaded].sort((a, b) => size(b) - size(a)).map((f) => ({ f, kb: size(f) / 1024, src: Object.entries(byFile.get(f).inputs ?? {}).sort((a, b) => b[1].bytesInOutput - a[1].bytesInOutput)[0]?.[0] }));
console.log(`JS the "${dialog}" window loads (static closure of main, bootstrap, dialog shell/bootstrap and its content): ${(total / 1024).toFixed(1)} kB in ${loaded.size} chunks`);
for (const r of rows.slice(0, 15)) console.log(`  ${r.kb.toFixed(1).padStart(8)} kB  ${r.f}  (${r.src ?? ''})`);
if (maxKb !== null && total / 1024 > maxKb) {
  console.error(`FAIL: ${(total / 1024).toFixed(1)} kB exceeds the ${maxKb} kB budget for the "${dialog}" window`);
  process.exit(1);
}
