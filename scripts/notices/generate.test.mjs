// Run: node --test scripts/notices
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { lf, lockProduction, nonPermissive, normalise, render, spdxIds } from './generate.mjs';

test('spdxIds splits expressions', () => {
  assert.deepEqual(spdxIds('(MIT OR Apache-2.0) AND Unicode-3.0'), ['MIT', 'Apache-2.0', 'Unicode-3.0']);
  assert.deepEqual(spdxIds(''), []);
});

test('lf unifies line endings and keeps everything else', () => {
  assert.equal(lf('\r\n  a  \r\nb\rc\n\n'), '\n  a  \nb\nc\n\n');
});

test('normalise unifies line endings and trailing space', () => {
  assert.equal(normalise('a  \r\nb\r\n\r\n'), 'a\nb');
});

test('lockProduction skips dev-only packages', () => {
  const lock = { packages: { '': {}, 'node_modules/a': { version: '1', license: 'MIT' }, 'node_modules/b': { version: '2', dev: true }, 'node_modules/c': { version: '3', devOptional: true } } };
  assert.deepEqual(lockProduction(lock).map((p) => p.name), ['a']);
});

test('nonPermissive flags only packages with no permissive option', () => {
  const pk = (license) => ({ kind: 'rust', name: 'x', version: '1', license, spdx: spdxIds(license) });
  assert.deepEqual(nonPermissive([pk('MIT OR LGPL-2.1-or-later'), pk('GPL-3.0-only')]), ['rust x@1: GPL-3.0-only']);
});

test('render deduplicates identical texts, is sorted and deterministic', () => {
  const mk = (name, kind, texts) => ({ kind, name, version: '1.0.0', license: 'MIT', spdx: ['MIT'], texts: texts.map((text) => ({ id: 'MIT', text })) });
  const rust = [mk('zeta', 'rust', ['MIT text A']), mk('alpha', 'rust', ['MIT text A'])];
  const npm = [mk('left', 'npm', ['MIT text B'])];
  const a = render(rust, npm);
  const b = render([...rust].reverse(), npm);
  assert.equal(a.markdown, b.markdown);
  assert.equal(a.counts.texts, 2);
  assert.ok(a.markdown.indexOf('- alpha 1.0.0') < a.markdown.indexOf('- zeta 1.0.0'));
  assert.match(a.markdown, /Used by \(2\):\n {2}alpha 1\.0\.0, zeta 1\.0\.0/);
});
