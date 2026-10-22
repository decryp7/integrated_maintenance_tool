#!/usr/bin/env node
// Builds THIRD_PARTY_NOTICES.md from cargo-about JSON (Rust crates) and the app's production npm
// dependencies (package-lock.json + each package's licence files). Pure function of its inputs and
// deterministic (sorted, no paths, no dates), so the output can be diffed and checked in CI.
//
// Usage: node generate.mjs --rust <cargo-about.json>... --app <app dir> --out <dir> [--out <dir>...]
//        node generate.mjs --check-only ... (prints the summary only)
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '..', '..');

/** Licences deny.toml accepts (cargo-about's `accepted` mirrors it); anything else is reported. */
const PERMISSIVE = new Set(['MIT', 'MIT-0', 'Apache-2.0', 'Apache-2.0 WITH LLVM-exception', 'BSD-2-Clause', 'BSD-3-Clause', 'ISC', 'Zlib', '0BSD', 'CC0-1.0', 'BSL-1.0', 'Unicode-3.0', 'Unlicense', 'MPL-2.0', 'CDLA-Permissive-2.0', 'OFL-1.1', 'Python-2.0', 'BlueOak-1.0.0']);
const LICENCE_FILE = /^(licen[cs]e|licen[cs]ing|copying|notice|unlicense)([-_.].*)?$/i;

/** Unifies line endings only, so a text embedded verbatim is the same on a CRLF (Windows) checkout. */
export function lf(text) {
  return text.replace(/\r\n?/g, '\n');
}

export function normalise(text) {
  return text.replace(/\r\n?/g, '\n').split('\n').map((l) => l.replace(/\s+$/, '')).join('\n').trim();
}
const cmp = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
const hash = (t) => createHash('sha256').update(t).digest('hex');

/** Splits an SPDX expression into its identifiers (OR/AND/WITH operands; "WITH" is kept attached). */
export function spdxIds(expr) {
  return expr
    .replace(/[()]/g, ' ')
    .split(/\s+(?:OR|AND)\s+/i)
    .map((s) => s.trim())
    .filter(Boolean);
}

/** Packages entries of the lock that ship in production (not dev-only). */
export function lockProduction(lock) {
  return Object.entries(lock.packages)
    .filter(([path, p]) => path.includes('node_modules/') && !p.dev && !p.devOptional)
    .map(([path, p]) => ({ path, name: path.slice(path.lastIndexOf('node_modules/') + 'node_modules/'.length), version: p.version, license: p.license ?? null }));
}

function npmPackages(appDir) {
  const lock = JSON.parse(readFileSync(join(appDir, 'package-lock.json'), 'utf8'));
  const warnings = [];
  const seen = new Set();
  const out = lockProduction(lock).filter((p) => !seen.has(`${p.name}@${p.version}`) && seen.add(`${p.name}@${p.version}`)).map((p) => {
    const dir = join(appDir, p.path);
    const pkgJson = existsSync(join(dir, 'package.json')) ? JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8')) : {};
    let license = p.license ?? (typeof pkgJson.license === 'string' ? pkgJson.license : pkgJson.license?.type) ?? 'UNKNOWN';
    const files = existsSync(dir) ? readdirSync(dir).filter((f) => LICENCE_FILE.test(f) && !/\.(js|ts|json|css)$/.test(f)).sort() : [];
    const texts = files
      .map((f) => ({ id: /apache/i.test(f) ? 'Apache-2.0' : /mit/i.test(f) ? 'MIT' : /^notice/i.test(f) ? 'NOTICE' : null, text: normalise(readFileSync(join(dir, f), 'utf8')) }))
      .filter((t) => t.text);
    if (!texts.length) warnings.push(`npm ${p.name}@${p.version}: no licence file found (declared ${license})`);
    return { kind: 'npm', name: p.name, version: p.version, license, texts, spdx: spdxIds(license) };
  });
  return { packages: out, warnings };
}

function rustCrates(files) {
  const crates = new Map(); // "name version" -> record
  const warnings = [];
  for (const file of files) {
    const j = JSON.parse(readFileSync(file, 'utf8'));
    for (const c of j.crates) {
      const p = c.package;
      if (!p.source) continue; // workspace crates are Debrief itself
      const key = `${p.name} ${p.version}`;
      if (!crates.has(key)) crates.set(key, { kind: 'rust', name: p.name, version: p.version, license: c.license || 'UNKNOWN', texts: [], spdx: spdxIds(c.license || '') });
    }
    for (const l of j.licenses) {
      const text = normalise(l.text);
      for (const u of l.used_by) {
        const rec = crates.get(`${u.crate.name} ${u.crate.version}`);
        if (rec && !rec.texts.some((t) => t.text === text)) rec.texts.push({ id: l.id, text });
      }
    }
  }
  for (const rec of crates.values()) if (!rec.texts.length) warnings.push(`rust ${rec.name}@${rec.version}: no licence text found (declared ${rec.license})`);
  return { packages: [...crates.values()], warnings };
}

function wrap(items, width = 100, indent = '') {
  const lines = [];
  let cur = '';
  for (const it of items) {
    const next = cur ? `${cur}, ${it}` : it;
    if (indent.length + next.length > width && cur) {
      lines.push(indent + cur + ',');
      cur = it;
    } else cur = next;
  }
  if (cur) lines.push(indent + cur);
  return lines;
}

export function render(rust, npm) {
  const all = [...rust, ...npm].sort((a, b) => cmp(a.kind, b.kind) || cmp(a.name, b.name) || cmp(a.version, b.version));
  // Deduplicate licence texts across all packages and give each a stable label like "MIT-3".
  const byHash = new Map();
  for (const p of all) {
    p.refs = p.texts.map((t) => {
      const h = hash(t.text);
      if (!byHash.has(h)) byHash.set(h, { text: t.text, id: t.id ?? p.spdx[0] ?? p.license, users: [] });
      byHash.get(h).users.push(`${p.name} ${p.version}`);
      return h;
    });
  }
  const counters = new Map();
  const texts = [...byHash.entries()].sort(([ha, a], [hb, b]) => cmp(a.id, b.id) || cmp(ha, hb));
  for (const [, t] of texts) {
    const n = (counters.get(t.id) ?? 0) + 1;
    counters.set(t.id, n);
    t.label = `${t.id}-${n}`;
  }
  const tally = new Map();
  for (const p of all) tally.set(p.license, (tally.get(p.license) ?? 0) + 1);

  const out = [];
  out.push('# Third-party notices', '');
  out.push('Debrief is dual-licensed MIT OR Apache-2.0 (see LICENSE-MIT and LICENSE-APACHE). It includes the third-party software listed below,');
  out.push('each under its own licence. This file is generated by scripts/notices/generate.sh; do not edit it by hand.', '');
  out.push('## Summary', '');
  out.push(`- Rust crates: ${rust.length}`, `- npm packages: ${npm.length}`, `- Distinct licence texts: ${texts.length}`);
  out.push('', 'Licence expressions (packages):', '');
  for (const [lic, n] of [...tally.entries()].sort(([a, x], [b, y]) => y - x || cmp(a, b))) out.push(`- ${lic}: ${n}`);
  for (const [heading, kind] of [['Rust crates', 'rust'], ['npm packages', 'npm']]) {
    const list = all.filter((p) => p.kind === kind);
    out.push('', `## ${heading} (${list.length})`, '');
    for (const p of list) out.push(`- ${p.name} ${p.version} — ${p.license} [${[...new Set(p.refs.map((h) => byHash.get(h).label))].join(', ') || 'no text'}]`);
  }
  out.push('', '## Licence texts', '');
  for (const [, t] of texts) {
    out.push(`### ${t.label}`, '');
    const users = wrap([...new Set(t.users)].sort(cmp), 96);
    out.push(`Used by (${new Set(t.users).size}):`, ...users.map((l) => `  ${l}`), '');
    out.push(...t.text.split('\n').map((l) => (l ? `    ${l}` : '')), '');
  }
  return { markdown: out.join('\n').replace(/\n+$/, '') + '\n', counts: { rust: rust.length, npm: npm.length, texts: texts.length, tally } };
}

export function nonPermissive(pkgs) {
  return pkgs.filter((p) => p.spdx.length && !p.spdx.some((id) => PERMISSIVE.has(id))).map((p) => `${p.kind} ${p.name}@${p.version}: ${p.license}`);
}

function main(argv) {
  const rustFiles = [];
  const outs = [];
  let app = join(repo, 'app');
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--rust') while (argv[i + 1] && !argv[i + 1].startsWith('--')) rustFiles.push(argv[++i]);
    else if (argv[i] === '--app') app = resolve(argv[++i]);
    else if (argv[i] === '--out') outs.push(resolve(argv[++i]));
  }
  const rust = rustCrates(rustFiles);
  const npm = npmPackages(app);
  const { markdown, counts } = render(rust.packages, npm.packages);
  for (const w of [...rust.warnings, ...npm.warnings]) console.error(`warning: ${w}`);
  const bad = nonPermissive([...rust.packages, ...npm.packages]);
  for (const b of bad) console.error(`NON-PERMISSIVE: ${b}`);
  const ts = (name, body) => `// Generated by scripts/notices/generate.sh. Do not edit.\n${body}`;
  for (const dir of outs) {
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, 'THIRD_PARTY_NOTICES.md'), markdown);
  }
  const legal = join(app, 'src', 'app', 'about', 'generated');
  mkdirSync(legal, { recursive: true });
  writeFileSync(join(legal, 'notices.generated.ts'), ts('notices', `export const THIRD_PARTY_NOTICES = ${JSON.stringify(markdown)};\n`));
  const mit = lf(readFileSync(join(repo, 'LICENSE-MIT'), 'utf8'));
  const apache = lf(readFileSync(join(repo, 'LICENSE-APACHE'), 'utf8'));
  writeFileSync(join(legal, 'licenses.generated.ts'), ts('licenses', `export const LICENSE_MIT = ${JSON.stringify(mit)};\nexport const LICENSE_APACHE = ${JSON.stringify(apache)};\n`));
  console.log(`notices: ${counts.rust} Rust crates, ${counts.npm} npm packages, ${counts.texts} distinct licence texts, ${counts.tally.size} licence expressions`);
  if (bad.length) process.exitCode = 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main(process.argv.slice(2));
