#!/usr/bin/env node
// Fails on hard-coded colours, font families and font sizes in the UI's styles (ADR 0018): components read only
// theme tokens (var(--…)), so every theme and scheme restyles all of them. Also fails on a var(--x) that nothing
// defines (a typo, or a token that was removed), which would silently fall back. Checks the `styles:` of components, the
// shared style modules (`*-styles.ts`) and the stylesheets under src/. The token values live in the theme data
// (src/app/theme/themes/*.theme.ts), which is TypeScript data, not styles, so it is not scanned.
//
// Usage: node scripts/lint-styles.mjs [src dir]   (part of `npm run lint`)
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

/** Files allowed to keep raw values, and why. Keep this list short. */
export const ALLOWED = new Map([
  // Printing: black on white whatever the theme.
  ['app/help/cheat-sheet.ts', 'cheat sheet print styles'],
]);

const NAMED = 'white|black|red|green|blue|yellow|orange|purple|gray|grey|pink|silver|navy|teal|maroon|olive|lime|aqua|fuchsia|magenta|cyan|brown|gold|indigo|violet';
const RULES = [
  { id: 'hex colour', re: /(^|[\s:,(])#[0-9a-fA-F]{3,8}(?![\w-])/ },
  { id: 'colour function', re: /\b(rgba?|hsla?|hwb|lab|lch|oklab|oklch)\(/ },
  { id: 'named colour', re: new RegExp(`:\\s*[^;{}]*\\b(${NAMED})\\b`, 'i') },
  { id: 'font size', re: /font-size\s*:\s*-?[\d.]+(px|pt|rem)\b/ },
  { id: 'font family', re: /font-family\s*:\s*(?!\s*(var\(--font-|inherit\b))/ },
  // The size in a `font:` shorthand (a line height after `/` may be a length).
  { id: 'font shorthand', re: /(^|[\s;{])font\s*:[^;]*(?<![/\d.])[\d.]+(px|pt|rem)\b/ },
];

/** Removes comments (keeping line breaks so line numbers stay right). */
export function stripComments(css) {
  return css.replace(/\/\*[\s\S]*?\*\//g, (c) => c.replace(/[^\n]/g, ' '));
}

/** The problems in one block of CSS: `{ line, rule, text }`, lines 1-based within the block. */
export function cssProblems(css) {
  const out = [];
  stripComments(css)
    .split('\n')
    .forEach((text, i) => {
      for (const r of RULES) if (r.re.test(text)) out.push({ line: i + 1, rule: r.id, text: text.trim() });
    });
  return out;
}

/** The style blocks of a TypeScript file with the line each starts on: component `styles:` and, in `*-styles.ts` modules, every template literal. */
export function styleBlocks(source, isStyleModule) {
  const blocks = [];
  const literal = /`((?:[^`\\]|\\.)*)`/y;
  const push = (at) => {
    literal.lastIndex = at;
    const m = literal.exec(source);
    if (!m) return at + 1;
    blocks.push({ css: m[1], line: source.slice(0, at + 1).split('\n').length });
    return literal.lastIndex;
  };
  if (isStyleModule) {
    for (let i = source.indexOf('`'); i >= 0; i = source.indexOf('`', push(i)));
    return blocks;
  }
  for (const m of source.matchAll(/styles\s*:\s*/g)) {
    let i = m.index + m[0].length;
    if (source[i] === '`') push(i);
    else if (source[i] === '[') {
      // An array of styles: every literal up to the closing bracket (shared style constants are checked where defined).
      for (i++; i < source.length && source[i] !== ']'; ) i = source[i] === '`' ? push(i) : i + 1;
    }
  }
  return blocks;
}

/** The problems of one file (by path relative to src, `/`-separated). */
export function fileProblems(rel, source) {
  if (ALLOWED.has(rel)) return [];
  if (/\.(s?css)$/.test(rel)) return cssProblems(source).map((p) => ({ ...p, file: rel }));
  const isStyleModule = /-styles\.ts$/.test(rel);
  return styleBlocks(source, isStyleModule).flatMap((b) => cssProblems(b.css).map((p) => ({ ...p, line: p.line + b.line - 1, file: rel })));
}

function* walk(dir) {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) yield* walk(p);
    else if (/\.(ts|s?css)$/.test(e.name) && !/\.spec\.ts$/.test(e.name)) yield p;
  }
}

/** Custom properties of libraries (Material, Dockview), defined outside src/. */
const FOREIGN = /^(mat|mdc|dv)-/;

/**
 * Custom properties a source defines: the keys of a theme's token sets (`'bg-1': '#…'` in `*.theme.ts`), CSS
 * declarations (`--x: …`), template bindings (`[style.--x]`) and `setProperty('--x', …)`.
 */
export function definedProperties(rel, source) {
  const names = new Set();
  const add = (re) => {
    for (const m of source.matchAll(re)) names.add(m[1]);
  };
  if (/\.theme\.ts$/.test(rel)) add(/(?:^\s*|[{,]\s*)'?([a-z][a-z0-9-]*)'?\s*:\s*['"]/gm);
  add(/--([a-zA-Z][\w-]*)\s*:/g);
  add(/\[style\.--([\w-]+)(?:\.[a-z%]+)?\]/g);
  add(/setProperty\(\s*['"`]--([\w-]+)/g);
  return names;
}

/** Custom properties a style block reads: `var(--x`. */
export function usedProperties(css) {
  return [...stripComments(css).matchAll(/var\(\s*--([\w-]+)/g)].map((m) => m[1]);
}

/** Style values that read a custom property nobody defines (a typo, or a token that was removed). */
export function unknownTokens(files) {
  const defined = new Set();
  for (const { rel, source } of files) for (const n of definedProperties(rel, source)) defined.add(n);
  const out = [];
  for (const { rel, source } of files) {
    if (ALLOWED.has(rel)) continue;
    const blocks = /\.(s?css)$/.test(rel) ? [{ css: source, line: 1 }] : styleBlocks(source, /-styles\.ts$/.test(rel));
    for (const b of blocks)
      stripComments(b.css)
        .split('\n')
        .forEach((text, i) => {
          for (const name of usedProperties(text))
            if (!defined.has(name) && !FOREIGN.test(name)) out.push({ file: rel, line: b.line + i, rule: 'unknown token', text: `--${name}` });
        });
  }
  return out;
}

export function lint(srcDir) {
  const out = [];
  const files = [];
  for (const file of walk(srcDir)) {
    const rel = relative(srcDir, file).split(sep).join('/');
    const source = readFileSync(file, 'utf8');
    files.push({ rel, source });
    out.push(...fileProblems(rel, source));
  }
  return [...out, ...unknownTokens(files)];
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const here = fileURLToPath(new URL('.', import.meta.url));
  const src = resolve(process.argv[2] ?? join(here, '..', 'src'));
  const problems = lint(src);
  for (const p of problems) console.error(`src/${p.file}:${p.line}: ${p.rule}: ${p.text}`);
  if (problems.length) {
    console.error(`\n${problems.length} style problem(s). Use a theme token (var(--…)); tokens are defined in src/app/theme/themes.`);
    process.exitCode = 1;
  } else console.log('lint-styles: no hard-coded colours, font families or font sizes, no unknown tokens');
}
