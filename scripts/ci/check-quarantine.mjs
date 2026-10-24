#!/usr/bin/env node
// Fails when a quarantine entry in docs/testing/quarantine.md has no valid `Expires` date (YYYY-MM-DD), is more than
// 6 months out, or has already passed (Paddock T11): a quarantined test is fixed or consciously renewed.
//   node scripts/ci/check-quarantine.mjs [file] [--today YYYY-MM-DD]
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const DEFAULT = resolve(dirname(fileURLToPath(import.meta.url)), '../../docs/testing/quarantine.md');
const MAX_DAYS = 190;

const isDate = (s) => /^\d{4}-\d{2}-\d{2}$/.test(s) && !Number.isNaN(Date.parse(`${s}T00:00:00Z`)) && new Date(`${s}T00:00:00Z`).toISOString().startsWith(s);
const days = (a, b) => Math.round((Date.parse(`${b}T00:00:00Z`) - Date.parse(`${a}T00:00:00Z`)) / 86_400_000);

/** Problems with the quarantine table in `md` as of `today` (YYYY-MM-DD); empty when every entry is current. */
export function problems(md, today) {
  const out = [];
  const lines = md.split(/\r?\n/);
  const head = lines.findIndex((l) => /^\|\s*Test\s*\|/i.test(l));
  if (head < 0) return ['no quarantine table (a header row starting with "| Test |")'];
  const cols = lines[head].split('|').slice(1, -1).map((c) => c.trim().toLowerCase());
  const exp = cols.indexOf('expires');
  if (exp < 0) return ['the quarantine table has no "Expires" column'];
  for (let i = head + 2; i < lines.length && lines[i].startsWith('|'); i++) {
    const cells = lines[i].split('|').slice(1, -1).map((c) => c.trim());
    const name = cells[0] ?? `row ${i + 1}`;
    const date = cells[exp] ?? '';
    if (!isDate(date)) out.push(`${name}: "Expires" must be a date (YYYY-MM-DD), got "${date}"`);
    else if (date < today) out.push(`${name}: expired on ${date}; fix the test or renew the date with a reason`);
    else if (days(today, date) > MAX_DAYS) out.push(`${name}: expires ${date}, more than 6 months out`);
  }
  return out;
}

export function main(argv, log = console) {
  const i = argv.indexOf('--today');
  const today = i >= 0 ? argv[i + 1] : new Date().toISOString().slice(0, 10);
  const file = argv.find((a, k) => !a.startsWith('--') && argv[k - 1] !== '--today') ?? DEFAULT;
  const found = problems(readFileSync(file, 'utf8'), today);
  for (const p of found) log.error(`quarantine: ${p}`);
  return found.length ? 1 : 0;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) process.exit(main(process.argv.slice(2)));
