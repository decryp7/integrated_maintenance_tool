#!/usr/bin/env node
// Renames the product IMT -> Debrief across the whole repository (see docs/rename-plan.md for the mapping and its rationale).
//
//   node scripts/rename-to-debrief.mjs --dry-run     per-file summary, changes nothing
//   node scripts/rename-to-debrief.mjs               apply (needs a clean tree), then run the post steps
//   node scripts/rename-to-debrief.mjs --no-post     apply but skip rustfmt / Cargo.lock refresh / sample regeneration
//   node scripts/rename-to-debrief.mjs --icons DIR   icon set to install (default ../suite-icons/debrief)
//
// Idempotent: once applied there is nothing left to match, so a second run changes nothing. No dependencies beyond Node and git.
// It never commits; review `git status` and commit the result yourself.

import { execFileSync, spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, rmdirSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const DRY = args.includes('--dry-run');
const NO_POST = args.includes('--no-post');
const FORCE_DIRTY = args.includes('--force-dirty');
const iconsArg = args.indexOf('--icons');
const ICONS = resolve(iconsArg >= 0 ? args[iconsArg + 1] : join(ROOT, '..', 'suite-icons', 'debrief'));

// ---------------------------------------------------------------------------------------------------------------------
// Deny list: nothing in these places is touched (paths are repo-relative, forward slashes).
// ---------------------------------------------------------------------------------------------------------------------

/** Files whose content is never rewritten. */
const DENY_CONTENT = [
  'scripts/rename-to-debrief.mjs', // this script names the old product on purpose
  'docs/rename-plan.md', // documents the old names on purpose
  'CHANGELOG.md', // generated history (git-cliff) describes what the product was called then
];

/** Lines whose content is never rewritten (hashes that happen to contain "imt" letters). */
const DENY_LINE = [/"integrity":\s*"sha\d+-/, /^checksum = "/];

/**
 * Tokens that contain "imt" but are protected from the rewrite. The report fonts embed their family name in the font
 * file itself (`name` table), and the PDF tests assert it; renaming the strings without rebuilding the fonts would
 * break them. Rebuild them (crates/imt-pdf/fonts/build-fonts.sh) in a separate change if the family should be renamed.
 */
const PROTECTED_TOKEN = /^IMT_?Report_?(?:Sans|Mono)/i;

/**
 * Edits after the rewrite for places where the new name has another LENGTH than the old one and a test pins the length.
 * Each is [file, text after the rewrite, replacement]. Reported when neither form is found (the code moved: update it).
 */
const FIXUPS = [
  // `container_name` is "debrief.plugin." + 16 hex digits (was "imt.plugin." + 16 = 27).
  ['crates/debrief-plugin-api/src/sandbox.rs', 'a.len() == 27, "{a}"', 'a.len() == 31, "{a}"'],
];

// ---------------------------------------------------------------------------------------------------------------------
// Phrase rules, applied before the token rule. Longest and most specific first.
// ---------------------------------------------------------------------------------------------------------------------

const PHRASES = [
  // "IMT — Integrated Maintenance Tool for analysing ..." (crate descriptions) -> the one-line description.
  [/IMT (?:—|\\u\{2014\}|-) integrated maintenance tool for analysing logs and diagnostic files/g, 'Debrief — an IDE for analysing logs and diagnostic files'],
  [/integrated maintenance tool for analysing logs and diagnostic files/g, 'IDE for analysing logs and diagnostic files'],
  // Window title / heading: "IMT — Integrated Maintenance Tool" -> "Debrief".
  [/IMT (?:—|\\u\{2014\}|-) Integrated Maintenance Tool/g, 'Debrief'],
  [/IMT \(Integrated Maintenance Tool\)/g, 'Debrief'],
  // The repository folder / URL segment (the user already renamed the Gitea repo to "debrief").
  [/integrated_maintenance_tool/g, 'debrief'],
  [/Integrated Maintenance Tool/gi, 'Debrief'],
];

// ---------------------------------------------------------------------------------------------------------------------
// Token rule: every "imt" (any case) that starts a word or a camelCase hump.
//   imt  -> debrief      (crate names imt-core, modules imt_core, selectors imt-panel, icon namespace imt:, camelCase imtGhost)
//   Imt  -> Debrief      (types ImtBackend, StoppedByImt)
//   IMT  -> Debrief      when it stands alone in prose or as a name        ("IMT", "IMT's", "IMT-ready")
//   IMT_ -> DEBRIEF_     inside an ALL-CAPS identifier                       (IMT_E2E_PORT, IMTPLUGIN, STOPPED_BY_IMT)
// Matches inside other words (a letter directly before a lowercase "imt") are left alone and reported as "unmapped".
// ---------------------------------------------------------------------------------------------------------------------

const isLetter = (c) => c !== undefined && /[A-Za-z]/.test(c);
const isLower = (c) => c !== undefined && /[a-z]/.test(c);

/** Rewrites the tokens in `text`. Returns the new text, the replacement count and the matches left alone. */
export function transformText(text) {
  for (const [re, to] of PHRASES) text = text.replace(re, to);
  let count = 0;
  const unmapped = [];
  let out = '';
  let last = 0;
  const re = /imt/gi;
  let m;
  while ((m = re.exec(text)) !== null) {
    const i = m.index;
    const word = m[0];
    const prev = text[i - 1];
    const prev2 = text[i - 2];
    // Escape sequences such as "\nimt-store-worker.exe" or "\rIMT": the letter after a backslash is not part of the word.
    const afterEscape = isLetter(prev) && prev2 === '\\';
    const inWord = isLetter(prev) && !afterEscape;
    if (PROTECTED_TOKEN.test(text.slice(i, i + 20))) continue;
    // A letter before the match is fine only at a camelCase hump: "fooImtBar", "StoppedByImt".
    if (inWord && !(word === 'Imt' && isLower(prev))) {
      unmapped.push(text.slice(Math.max(0, i - 12), i + 15).replace(/\s+/g, ' '));
      continue;
    }
    let to;
    if (word === 'imt') to = 'debrief';
    else if (word === 'Imt') to = 'Debrief';
    else if (word === 'IMT') {
      // The identifier run around the match decides between prose and ALL_CAPS.
      let s = i;
      while (s > 0 && /[A-Za-z0-9_]/.test(text[s - 1]) && !(s - 1 > 0 && text[s - 2] === '\\' && /[a-z]/.test(text[s - 1]))) s--;
      if (afterEscape) s = i;
      let e = i + 3;
      while (e < text.length && /[A-Za-z0-9_]/.test(text[e])) e++;
      const run = text.slice(s, e);
      to = run !== 'IMT' && run === run.toUpperCase() ? 'DEBRIEF' : 'Debrief';
    } else {
      unmapped.push(text.slice(Math.max(0, i - 12), i + 15));
      continue;
    }
    out += text.slice(last, i) + to;
    last = i + 3;
    count++;
  }
  out += text.slice(last);
  if (count > 0 || out !== text) {
    // "an IMT" -> "an Debrief" is wrong.
    out = out.replace(/\b([Aa])n (?=[Dd]ebrief)/g, '$1 ');
  }
  return { text: out, count, unmapped };
}

/** The new path for a repo-relative path (segments are rewritten independently). */
export function transformPath(p) {
  return p
    .split('/')
    .map((seg) => transformText(seg).text)
    .join('/');
}

// ---------------------------------------------------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------------------------------------------------

function git(...a) {
  return execFileSync('git', a, { cwd: ROOT, encoding: 'utf8', maxBuffer: 1 << 28 });
}

function isBinary(buf) {
  const n = Math.min(buf.length, 8000);
  for (let i = 0; i < n; i++) if (buf[i] === 0) return true;
  return false;
}

/** Splits into lines keeping terminators so that DENY_LINE can skip single lines without disturbing the rest. */
function rewriteContent(text) {
  let count = 0;
  const unmapped = [];
  const parts = text.split(/(?<=\n)/);
  const out = parts.map((line) => {
    if (DENY_LINE.some((re) => re.test(line))) return line;
    const r = transformText(line);
    count += r.count;
    unmapped.push(...r.unmapped);
    return r.text;
  });
  return { text: out.join(''), count, unmapped };
}

function main() {
  if (!existsSync(join(ROOT, '.git'))) throw new Error(`not a git checkout: ${ROOT}`);
  if (!DRY && !FORCE_DIRTY && git('status', '--porcelain').trim() !== '') {
    throw new Error('the working tree is not clean: commit or stash first (or pass --force-dirty)');
  }
  const files = git('ls-files', '-z').split('\0').filter(Boolean);
  const summary = [];
  const allUnmapped = [];
  let total = 0;

  // 1. Contents.
  for (const f of files) {
    if (DENY_CONTENT.includes(f)) continue;
    const abs = join(ROOT, f);
    if (!existsSync(abs)) continue;
    const buf = readFileSync(abs);
    if (isBinary(buf)) continue;
    const text = buf.toString('utf8');
    if (!Buffer.from(text, 'utf8').equals(buf)) continue; // not valid UTF-8: leave it
    const r = rewriteContent(text);
    for (const [file, from, to] of FIXUPS) {
      if (file !== transformPath(f)) continue;
      if (r.text.includes(from)) {
        r.text = r.text.replace(from, to);
        r.count++;
      } else if (!r.text.includes(to)) allUnmapped.push(`${f}: fixup not applicable: ${from}`);
    }
    for (const u of r.unmapped) allUnmapped.push(`${f}: ...${u}...`);
    if (r.text !== text) {
      total += r.count;
      summary.push(`${String(r.count).padStart(5)}  ${f}`);
      if (!DRY) writeFileSync(abs, r.text);
    }
  }

  // 2. Paths: directories deepest first, then files, all with `git mv`.
  const renames = [];
  const dirs = new Set();
  for (const f of files) {
    const parts = f.split('/');
    for (let i = 1; i < parts.length; i++) dirs.add(parts.slice(0, i).join('/'));
  }
  const dirMoves = [...dirs]
    .filter((d) => transformPath(d.split('/').pop()) !== d.split('/').pop())
    .sort((a, b) => b.split('/').length - a.split('/').length);
  for (const d of dirMoves) {
    const base = d.split('/').pop();
    const to = [...d.split('/').slice(0, -1), transformPath(base)].join('/');
    renames.push([d, to]);
  }
  const fileMoves = [];
  for (const f of files) {
    const parts = f.split('/');
    const base = parts.pop();
    const newBase = transformPath(base);
    if (newBase !== base) {
      const dir = parts.map(transformPath);
      fileMoves.push([[...dir, base].join('/'), [...dir, newBase].join('/')]);
    }
  }
  for (const [from, to] of renames) {
    if (!DRY) {
      if (existsSync(join(ROOT, to))) throw new Error(`rename target exists: ${to} (from ${from})`);
      git('mv', from, to);
    }
  }
  for (const [from, to] of fileMoves) {
    if (!DRY) {
      if (existsSync(join(ROOT, to))) throw new Error(`rename target exists: ${to} (from ${from})`);
      mkdirSync(dirname(join(ROOT, to)), { recursive: true });
      git('mv', from, to);
    }
  }

  // 3. Icons.
  const iconPlan = [
    ['tauri', 'app/src-tauri/icons', null],
    ['tray/macos/debrief-tray-18ptTemplate@2x.png', 'app/src-tauri/icons/tray-template.png', 'file'],
    ['debrief-windows.svg', 'app/src-tauri/icons-src/debrief-icon.svg', 'file'],
    ['small/debrief-32.svg', 'app/src-tauri/icons-src/debrief-icon-small.svg', 'file'],
    ['tray/svg/debrief-tray-template.svg', 'app/src-tauri/icons-src/debrief-tray-template.svg', 'file'],
    ['debrief-macos.svg', 'assets/icon/debrief-icon.svg', 'file'],
    ['debrief-macos-1024.png', 'assets/icon/debrief-icon-1024.png', 'file'],
  ];
  const iconNotes = [];
  if (!existsSync(ICONS)) {
    iconNotes.push(`icon set not found at ${ICONS}: skipped (pass --icons DIR)`);
  } else {
    for (const [src, dst, kind] of iconPlan) {
      const s = join(ICONS, src);
      if (!existsSync(s)) {
        iconNotes.push(`missing in icon set: ${src}`);
        continue;
      }
      const pairs = kind === 'file' ? [[s, join(ROOT, dst)]] : readdirSync(s).map((n) => [join(s, n), join(ROOT, dst, n)]);
      for (const [a, b] of pairs) {
        const same = existsSync(b) && readFileSync(a).equals(readFileSync(b));
        if (!same) {
          iconNotes.push(`icon  ${b.slice(ROOT.length + 1)}`);
          if (!DRY) {
            mkdirSync(dirname(b), { recursive: true });
            copyFileSync(a, b);
          }
        }
      }
    }
  }

  // Report.
  console.log(summary.sort().join('\n'));
  console.log(`\n${DRY ? 'would rewrite' : 'rewrote'} ${total} occurrences in ${summary.length} files`);
  console.log(`${DRY ? 'would move' : 'moved'} ${renames.length} directories and ${fileMoves.length} files with git mv`);
  if (DRY) {
    for (const [a, b] of renames) console.log(`  dir   ${a} -> ${b}`);
    for (const [a, b] of fileMoves.slice(0, 40)) console.log(`  file  ${a} -> ${b}`);
    if (fileMoves.length > 40) console.log(`  ... and ${fileMoves.length - 40} more file moves`);
  }
  for (const n of iconNotes) console.log(`  ${n}`);
  if (allUnmapped.length) {
    console.log(`\n${allUnmapped.length} matches inside other words were left alone (check they are not product names):`);
    for (const u of allUnmapped.slice(0, 60)) console.log(`  ${u}`);
  }

  // 4. Post steps (formatting and generated files that depend on the new names).
  if (!DRY && !NO_POST) postSteps();
  if (!DRY) residualReport();
}

/** Lists what still matches /imt/i after the rewrite (expected: only deliberate leftovers). */
function residualReport() {
  const left = [];
  for (const f of git('ls-files', '-z').split('\0').filter(Boolean)) {
    if (DENY_CONTENT.includes(f) || !existsSync(join(ROOT, f))) continue;
    const buf = readFileSync(join(ROOT, f));
    if (isBinary(buf)) continue;
    buf
      .toString('utf8')
      .split('\n')
      .forEach((line, n) => {
        if (DENY_LINE.some((re) => re.test(line))) return;
        if (/imt/i.test(line.replace(/IMT_?Report_?(?:Sans|Mono)/gi, ''))) left.push(`${f}:${n + 1}: ${line.trim().slice(0, 110)}`);
      });
  }
  console.log(`\nresidual "imt" matches after the rewrite: ${left.length}`);
  for (const l of left.slice(0, 40)) console.log(`  ${l}`);
  const stray = git('ls-files', '-z').split('\0').filter((f) => f && /imt/i.test(f) && !/IMT_?Report_?(?:Sans|Mono)/i.test(f));
  if (stray.length) console.log(`paths still containing "imt": ${stray.join(', ')}`);
}

function run(cmd, cmdArgs, opts = {}) {
  console.log(`$ ${cmd} ${cmdArgs.join(' ')}`);
  const r = spawnSync(cmd, cmdArgs, { cwd: ROOT, stdio: 'inherit', ...opts });
  if (r.status !== 0) console.log(`  (exit ${r.status}: continue, see docs/rename-plan.md, "Manual steps")`);
}

function postSteps() {
  // Format only the Rust files the rename touched (longer names reflow lines); never the whole workspace.
  const changed = git('diff', '--name-only', '--diff-filter=AMR', 'HEAD').split('\n').filter((f) => f.endsWith('.rs'));
  for (let i = 0; i < changed.length; i += 200) run('rustfmt', ['--edition', '2021', '--config-path', ROOT, ...changed.slice(i, i + 200)]);
  // Cargo.lock keeps its package order sorted by name.
  run('cargo', ['update', '--workspace', '--offline']);
  // Samples, share bundles, snapshots and the zip fixture embed the product name: regenerate them with their generator.
  run('cargo', ['xtask', 'gen-samples']);
}

try {
  main();
} catch (e) {
  console.error(`rename-to-debrief: ${e.message}`);
  process.exit(1);
}
