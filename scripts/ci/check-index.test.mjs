import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { inlineElements, problemsOf, splashProblems } from './check-index.mjs';

const SPLASH =
  '<debrief-root><div class="debrief-splash" role="status"><img src="favicon.ico" alt="" width="48" height="48"><p>Starting Debrief&hellip;</p></div></debrief-root>';

test('inline styles and scripts are found', () => {
  const html = '<head><style>:root{}</style><link rel="stylesheet" href="s.css"></head><body><script>x()</script></body>';
  assert.deepEqual(inlineElements(html), ['<style>', '<script>']);
});

test('external scripts and stylesheets pass', () => {
  assert.deepEqual(inlineElements('<link rel="stylesheet" href="s.css"><script src="main.js" type="module"></script>'), []);
});

test('the splash must sit inside the root element', () => {
  assert.deepEqual(splashProblems(SPLASH), []);
  assert.deepEqual(problemsOf(`<body>${SPLASH}<script src="m.js"></script></body>`), []);
  assert.match(splashProblems('<debrief-root></debrief-root>')[0], /no \.debrief-splash/);
  assert.match(splashProblems('<body></body>')[0], /no <debrief-root>/);
  assert.ok(splashProblems(SPLASH.replace('<p>', '<p style="x">').replace('<img', '<img style="y"')).some((p) => /inline style/.test(p)));
});

test('the source index.html has the splash and no inline elements', () => {
  const html = readFileSync(new URL('../../app/src/index.html', import.meta.url), 'utf8');
  assert.deepEqual(problemsOf(html), []);
});
