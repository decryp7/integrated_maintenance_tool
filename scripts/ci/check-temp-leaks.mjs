#!/usr/bin/env node
// Fails when test runs leave `debrief-*` / `imt-*` folders in the OS temp directory (docs/testing/temp-leaks.md).
//   node scripts/ci/check-temp-leaks.mjs --snapshot leaks.json   (before the tests: records what is already there)
//   node scripts/ci/check-temp-leaks.mjs --check leaks.json      (after the tests: exits 1 naming the new ones)
//   node scripts/ci/check-temp-leaks.mjs                         (no baseline: every matching folder counts)
// `--dir <path>` scans that folder instead of os.tmpdir().

import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { pathToFileURL } from 'node:url';

export const PATTERN = /^(debrief|imt)-/;

/** Names in `dir` that look like Debrief test scratch folders, sorted; an unreadable folder has none. */
export function listLeaks(dir = tmpdir()) {
  try {
    return readdirSync(dir).filter((n) => PATTERN.test(n)).sort();
  } catch {
    return [];
  }
}

/** The names in `current` that are not in `baseline`. */
export function newLeaks(current, baseline = []) {
  const known = new Set(baseline);
  return current.filter((n) => !known.has(n));
}

function readBaseline(file) {
  try {
    const v = JSON.parse(readFileSync(file, 'utf8'));
    return Array.isArray(v) ? v.filter((x) => typeof x === 'string') : [];
  } catch {
    return [];
  }
}

export function main(argv, log = console) {
  const opt = (name) => {
    const i = argv.indexOf(name);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const dir = opt('--dir') ?? tmpdir();
  const current = listLeaks(dir);
  const snapshot = opt('--snapshot');
  if (snapshot) {
    writeFileSync(snapshot, JSON.stringify(current));
    log.log(`recorded ${current.length} existing debrief-*/imt-* folder(s) in ${dir}`);
    return 0;
  }
  const check = opt('--check');
  const leaked = newLeaks(current, check ? readBaseline(check) : []);
  if (leaked.length === 0) {
    log.log(`no debrief-*/imt-* folders left in ${dir}`);
    return 0;
  }
  log.error(`${leaked.length} test folder(s) left in ${dir} (tests must use tempfile::TempDir or a guard that cleans on panic):`);
  for (const n of leaked) log.error(`  ${n}`);
  return 1;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) process.exit(main(process.argv.slice(2)));
