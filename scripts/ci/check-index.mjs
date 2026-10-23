#!/usr/bin/env node
// Checks the built index.html (app/dist/app/browser, plus any locale folder next to it):
//   1. no inline <style> or <script> elements. Tauri adds a nonce or hash for each to the CSP, and a style-src with a
//      nonce makes browsers ignore 'unsafe-inline', which blocks the <style> elements Angular adds at run time.
//      (angular.json keeps "inlineCritical": false for this reason.)
//   2. the start-up splash (logo + "Starting Debrief…") sits inside <debrief-root> so the window is never empty while
//      the UI loads (Paddock lesson; the styles are in src/styles.scss).
//   node scripts/ci/check-index.mjs [path/to/index.html ...]   (default: app/dist/app/browser/**/index.html)
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

/** Inline `<style>` / `<script>` opening tags (a script with a `src` is external and fine). */
export function inlineElements(html) {
  const found = [];
  for (const m of html.matchAll(/<(style|script)\b([^>]*)>/gi)) {
    if (m[1].toLowerCase() === 'style' || !/\bsrc\s*=/.test(m[2])) found.push(`<${m[1]}${m[2]}>`);
  }
  return found;
}

/** Problems with the start-up splash (empty when it is there and sits inside the root element). */
export function splashProblems(html) {
  const root = html.match(/<debrief-root\b[^>]*>([\s\S]*?)<\/debrief-root>/i);
  if (!root) return ['no <debrief-root> element'];
  const out = [];
  if (!/class="[^"]*\bdebrief-splash\b/.test(root[1])) out.push('<debrief-root> has no .debrief-splash element');
  if (!/Starting Debrief(…|&hellip;|&#8230;)/.test(root[1])) out.push('the splash does not say "Starting Debrief…"');
  if (!/<img\b[^>]*\bsrc=/.test(root[1])) out.push('the splash has no logo <img>');
  if (/\bstyle\s*=/.test(root[1])) out.push('the splash uses an inline style attribute (CSP): style it from styles.scss');
  return out;
}

export function problemsOf(html) {
  const inline = inlineElements(html);
  const out = inline.length
    ? [`inline elements break the app's styles under Tauri's CSP: ${inline.join(' ')} (keep "inlineCritical": false in angular.json)`]
    : [];
  return [...out, ...splashProblems(html)];
}

function defaultFiles() {
  const base = resolve(dirname(fileURLToPath(import.meta.url)), '../../app/dist/app/browser');
  const files = [join(base, 'index.html')];
  if (existsSync(base)) {
    for (const d of readdirSync(base, { withFileTypes: true })) {
      if (d.isDirectory() && existsSync(join(base, d.name, 'index.html'))) files.push(join(base, d.name, 'index.html'));
    }
  }
  return files;
}

export function main(argv, log = console) {
  const files = argv.length ? argv : defaultFiles();
  let bad = 0;
  for (const f of files) {
    if (!existsSync(f)) {
      log.error(`${f}: not found (run the frontend build first)`);
      bad++;
      continue;
    }
    for (const p of problemsOf(readFileSync(f, 'utf8'))) {
      log.error(`${f}: ${p}`);
      bad++;
    }
  }
  return bad ? 1 : 0;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) process.exit(main(process.argv.slice(2)));
