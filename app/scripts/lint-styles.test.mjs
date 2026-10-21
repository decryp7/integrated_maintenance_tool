// Run: node --test scripts/lint-styles.test.mjs
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { cssProblems, definedProperties, fileProblems, lint, stripComments, styleBlocks, unknownTokens } from './lint-styles.mjs';

const rules = (css) => cssProblems(css).map((p) => p.rule);

test('flags raw colours in every notation', () => {
  assert.deepEqual(rules('a { color: #fff; }'), ['hex colour']);
  assert.deepEqual(rules('a { border: 1px solid #12345678; }'), ['hex colour']);
  assert.deepEqual(rules('a { background: rgba(0, 0, 0, 0.4); }'), ['colour function']);
  assert.deepEqual(rules('a { color: hsl(10 20% 30%); }'), ['colour function']);
  assert.deepEqual(rules('a { color: oklch(0.5 0.1 200); }'), ['colour function']);
  assert.deepEqual(rules('a { color: white; }'), ['named colour']);
  assert.deepEqual(rules('a { color: var(--x, #000); }'), ['hex colour']);
});

test('accepts tokens, keywords and colour mixes of tokens', () => {
  assert.deepEqual(rules('a { color: var(--text); background: transparent; border-color: currentColor; }'), []);
  assert.deepEqual(rules('a { background: color-mix(in srgb, var(--accent) 20%, transparent); }'), []);
  assert.deepEqual(rules('a { color: inherit; } #main { display: block; }'), []);
  assert.deepEqual(rules('.white-space { white-space: nowrap; }'), []);
});

test('flags hard-coded font sizes and families, not tokens or relative sizes', () => {
  assert.deepEqual(rules('a { font-size: 12px; }'), ['font size']);
  assert.deepEqual(rules('a { font-size: 0.8rem; }'), ['font size']);
  assert.deepEqual(rules('a { font-size: var(--fs-sm); font-size: 90%; font-size: 1.2em; }'), []);
  assert.deepEqual(rules("a { font-family: 'Inter', sans-serif; }"), ['font family']);
  assert.deepEqual(rules('a { font-family: var(--font-mono); } b { font-family: inherit; }'), []);
  assert.deepEqual(rules('a { font: 600 13px/1.2 system-ui; }'), ['font shorthand']);
  assert.deepEqual(rules('a { font: var(--fs-md)/28px var(--font-mono); }'), []);
});

test('ignores comments and keeps line numbers', () => {
  assert.equal(stripComments('/* #fff\n */ a {}'), '       \n    a {}');
  assert.deepEqual(cssProblems('/* rgba(0,0,0) */\na {}\nb { color: #000; }'), [{ line: 3, rule: 'hex colour', text: 'b { color: #000; }' }]);
});

test('reads component styles (single and arrays) and style modules with their line numbers', () => {
  const src = "@Component({\n  template: `<b style=\"color: red\"></b>`,\n  styles: `\n    a { color: #fff; }\n  `,\n})\nclass A {}\n@Component({ styles: [SHARED, `b { color: red; }`] })\nclass B {}";
  const blocks = styleBlocks(src, false);
  assert.equal(blocks.length, 2);
  assert.equal(blocks[0].line, 3);
  const problems = fileProblems('app/x.ts', src);
  assert.deepEqual(problems.map((p) => [p.line, p.rule]), [[4, 'hex colour'], [8, 'named colour']]);
  assert.deepEqual(fileProblems('app/x-styles.ts', 'export const S = `a { color: #fff; }`;').map((p) => p.rule), ['hex colour']);
  assert.deepEqual(fileProblems('styles.scss', 'a {\n  color: #fff;\n}').map((p) => p.line), [2]);
});

test('skips allow-listed files and the template markup', () => {
  assert.deepEqual(fileProblems('app/help/cheat-sheet.ts', 'styles: `a { color: #fff; }`'), []);
  assert.deepEqual(fileProblems('app/y.ts', 'template: `<svg fill="#fff"></svg>`'), []);
});

test('walks a source tree, skipping specs', () => {
  const dir = mkdtempSync(join(tmpdir(), 'lint-styles-'));
  mkdirSync(join(dir, 'app'));
  writeFileSync(join(dir, 'app', 'a.ts'), 'styles: `a { color: #fff; }`');
  writeFileSync(join(dir, 'app', 'a.spec.ts'), 'styles: `a { color: #fff; }`');
  writeFileSync(join(dir, 'styles.scss'), ':root { --text: inherit; }\na { color: var(--text); }');
  assert.deepEqual(lint(dir).map((p) => p.file), ['app/a.ts']);
});

test('flags tokens nothing defines, and knows theme keys, declarations, bindings and setProperty', () => {
  const files = [
    { rel: 'app/theme/themes/a.theme.ts', source: "export const A = {\n  common: { 'fs-sm': '12px' },\n  'bg-1': '#fff',\n};" },
    { rel: 'app/x.ts', source: "template: `<i [style.--lane-c]=\"c\" [style.--row-h.px]=\"h\"></i>`, styles: `:host { --local: 1px; color: var(--bg-1); font-size: var(--fs-sm); width: var(--local); top: var(--row-h); fill: var(--lane-c); --mat-sys-x: var(--mat-sys-primary); }`" },
    { rel: 'app/y.ts', source: "el.style.setProperty('--set', '1'); const s = 0;\nstyles: `a { color: var(--set); }\nb { color: var(--txt); }`" },
  ];
  assert.deepEqual(unknownTokens(files).map((p) => [p.file, p.line, p.text]), [['app/y.ts', 3, '--txt']]);
  assert.deepEqual([...definedProperties('app/theme/themes/a.theme.ts', files[0].source)].sort(), ['bg-1', 'fs-sm']);
});
