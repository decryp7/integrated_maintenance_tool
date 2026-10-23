#!/usr/bin/env node
// Rule 28: `effect()` is for side effects on things outside the signal graph. An effect that writes state directly
// (`.set(`, `.update(`, `patchState(`) must say why it is not a computed/linkedSignal/resource, in a comment right above
// it that starts with `effect-state:` (one line, the reason). Fails on one without. Part of `npm run lint`.
//
// Usage: node scripts/lint-effects.mjs [src dir]
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const WRITE = /\.(set|update)\(|\bpatchState\(/;
export const TAG = 'effect-state:';

/** Blanks out string and comment contents (keeping offsets) so braces and parens inside them do not count. */
export function mask(src) {
  let out = '';
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    const n = src[i + 1];
    if (c === '/' && n === '/') {
      const e = src.indexOf('\n', i);
      const end = e < 0 ? src.length : e;
      out += ' '.repeat(end - i);
      i = end;
    } else if (c === '/' && n === '*') {
      const e = src.indexOf('*/', i + 2);
      const end = e < 0 ? src.length : e + 2;
      out += src.slice(i, end).replace(/[^\n]/g, ' ');
      i = end;
    } else if (c === "'" || c === '"' || c === '`') {
      let j = i + 1;
      while (j < src.length && src[j] !== c) j += src[j] === '\\' ? 2 : 1;
      out += c + src.slice(i + 1, j).replace(/[^\n]/g, ' ') + c;
      i = j + 1;
    } else {
      out += c;
      i++;
    }
  }
  return out;
}

/** The effects in a file that write state without the tag: `{ line, text }` (1-based). */
export function problems(src) {
  const masked = mask(src);
  const lines = src.split('\n');
  const out = [];
  for (const m of masked.matchAll(/(?<![.\w])effect\s*\(/g)) {
    let depth = 0;
    let end = m.index + m[0].length - 1;
    for (; end < masked.length; end++) {
      if (masked[end] === '(') depth++;
      else if (masked[end] === ')' && --depth === 0) break;
    }
    if (!WRITE.test(masked.slice(m.index, end))) continue;
    const line = masked.slice(0, m.index).split('\n').length;
    // The contiguous comment lines right above the statement.
    let above = line - 2;
    let tagged = false;
    while (above >= 0 && lines[above].trim().startsWith('//')) tagged ||= lines[above--].includes(TAG);
    if (!tagged) out.push({ line, text: lines[line - 1].trim() });
  }
  return out;
}

function* files(dir) {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) yield* files(p);
    else if (e.name.endsWith('.ts') && !e.name.endsWith('.spec.ts')) yield p;
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const root = resolve(process.argv[2] ?? 'src');
  let bad = 0;
  for (const f of files(root)) {
    for (const p of problems(readFileSync(f, 'utf8'))) {
      console.error(`${relative(process.cwd(), f).split(sep).join('/')}:${p.line}: effect writes state without a "// ${TAG} <reason>" comment (rule 28): ${p.text}`);
      bad++;
    }
  }
  if (bad) {
    console.error(`\n${bad} unjustified state-writing effect(s). Use computed/linkedSignal/resource, or justify the effect.`);
    process.exit(1);
  }
}
