import { test } from 'node:test';
import assert from 'node:assert/strict';
import { measure, packageOf, staticClosure, indexOutputs, windowEntries } from './window-weight.mjs';

const out = (entryPoint, bytes, imports = [], inputs = {}) => ({ entryPoint, bytes, imports, inputs });
const imp = (path, kind = 'import-statement') => ({ path, kind });
const outputs = {
  'dist/main.js': out('src/main.ts', 100, [imp('chunk-a.js'), imp('bootstrap.js', 'dynamic-import')], { 'src/main.ts': { bytesInOutput: 100 } }),
  'dist/bootstrap.js': out('src/bootstrap.ts', 200, [imp('chunk-a.js')], { 'src/bootstrap.ts': { bytesInOutput: 200 } }),
  'dist/app.js': out('src/app/app.ts', 300, [imp('chunk-b.js'), imp('lazy.js', 'dynamic-import')]),
  'dist/config.js': out('src/app/app.config.ts', 400),
  'dist/popout.js': out('src/app/popout/popout-shell.ts', 50),
  'dist/chunk-a.js': { bytes: 10, imports: [], inputs: { 'node_modules/@angular/core/x.mjs': { bytesInOutput: 10 } } },
  'dist/chunk-b.js': { bytes: 20, imports: [], inputs: { 'src/app/dock/dock.ts': { bytesInOutput: 20 } } },
  'dist/lazy.js': { bytes: 5000, imports: [], inputs: {} },
};

test('the main window counts static imports and skips dynamic ones', () => {
  const m = measure(outputs, 'main');
  assert.equal(m.total, 100 + 200 + 300 + 400 + 10 + 20);
  assert.equal(m.count, 6);
  assert.ok(!m.files.has('lazy.js'));
});
test('the pop-out swaps the app for its shell', () => {
  const m = measure(outputs, 'popout');
  assert.equal(m.total, 100 + 200 + 50 + 400 + 10);
});
test('a missing entry means the build is not current', () => {
  assert.equal(measure(outputs, 'about'), null);
});
test('packages group by npm package or src/app directory', () => {
  assert.equal(packageOf('node_modules/@angular/core/fesm2022/x.mjs'), '@angular/core');
  assert.equal(packageOf('node_modules/smol-toml/dist/a.js'), 'smol-toml');
  assert.equal(packageOf('src/app/dock/dock.ts'), 'src/app/dock');
  assert.equal(packageOf('src/main.ts'), 'src');
  const m = measure(outputs, 'main');
  assert.deepEqual(m.packages.find((p) => p.name === 'src/app/dock'), { name: 'src/app/dock', bytes: 20 });
});
test('the closure ignores unknown files and cycles', () => {
  const by = indexOutputs({ 'd/a.js': { bytes: 1, imports: [imp('b.js'), imp('missing.js')] }, 'd/b.js': { bytes: 1, imports: [imp('a.js')] } });
  assert.deepEqual([...staticClosure(by, ['a.js'])].sort(), ['a.js', 'b.js']);
});
test('a dialog window names its content chunk', () => {
  assert.ok(windowEntries('about').includes('src/app/about/about-dialog.ts'));
});
