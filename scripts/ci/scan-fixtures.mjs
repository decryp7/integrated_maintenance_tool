#!/usr/bin/env node
// Fails when samples or test fixtures contain log material that looks real (pasted logs are confidential: AGENTS.md,
// Paddock S04). Deliberately conservative: it flags only
//   - e-mail addresses whose domain is not reserved (example.*, *.test, *.invalid, localhost, ...),
//   - public IPv4 addresses (not private, loopback, link-local, documentation ranges, masks or version-like values),
//   - hostnames under company-internal suffixes (.corp, .lan, .intranet, .internal, .home, .local) that are not allowlisted.
// Anything else (public domains, user names, GUIDs) is left to review. Reviewed false positives go in
// scripts/ci/scan-fixtures.allow.json (documented in docs/testing/fixture-scan.md), never into a loosened regex.
//
//   node scripts/ci/scan-fixtures.mjs [root ...]      (default: samples and every crates/*/tests/fixtures)
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const MAX_BYTES = 4 * 1024 * 1024;
const SKIP_DIRS = new Set(['node_modules', 'target', '.git']);
const BINARY_EXT = /\.(zip|gz|tgz|7z|bz2|xz|zst|png|jpe?g|gif|ico|webp|pdf|dmp|mdmp|etl|evtx|db|duckdb|sqlite3?|wasm|bin|exe|dll|so|dylib|pcap|pcapng|jar)$/i;

const RESERVED_DOMAIN = /(^|\.)(example\.(com|org|net|edu)|example|test|invalid|localhost|local|localdomain)$/i;
// Lower-case only: DNS names are, and .NET/Java namespaces (Microsoft.Extensions.Internal) must not match.
const INTERNAL_SUFFIX = /(?<![\w.-])[a-z0-9][a-z0-9-]*(?:\.[a-z0-9][a-z0-9-]*)*\.(?:corp|lan|intranet|internal|home)(?![\w-])/g;
const EMAIL = /\b[A-Za-z0-9._%+-]+@([A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+)\b/g;
const IPV4 = /(?<![\w.])(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})(?![\w]|\.\d)/g;

/** Whether an IPv4 is one a sample may contain freely (non-public, documentation, mask or obviously synthetic). */
export function isHarmlessIp([a, b, c, d]) {
  if ([a, b, c, d].some((n) => n > 255)) return true; // not an address (a version or a counter)
  if (a === 0 || a === 10 || a === 127 || a >= 224) return true; // this-net, private, loopback, multicast/reserved/masks
  if (a === 172 && b >= 16 && b <= 31) return true;
  if (a === 192 && b === 168) return true;
  if (a === 169 && b === 254) return true;
  if (a === 100 && b >= 64 && b <= 127) return true; // CGNAT
  if (a === 192 && b === 0 && c === 2) return true; // TEST-NET-1
  if (a === 198 && b === 51 && c === 100) return true; // TEST-NET-2
  if (a === 203 && b === 0 && c === 113) return true; // TEST-NET-3
  if (a === 198 && (b === 18 || b === 19)) return true; // benchmarking
  if (b === c && c === d) return true; // 1.2.2.2 style placeholders... and 8.8.8.8 style well-known resolvers
  if (a === 1 && b === 2 && c === 3 && d === 4) return true;
  return false;
}

/** Findings in one file's text: `{line, kind, text}`. `allow` is the parsed allow list. */
export function scanText(text, allow = { values: [] }) {
  const ok = new Set(allow.values.map((v) => v.toLowerCase()));
  const out = [];
  const lines = text.split(/\r?\n/);
  lines.forEach((line, i) => {
    const hit = (kind, value) => {
      if (!ok.has(value.toLowerCase())) out.push({ line: i + 1, kind, text: value });
    };
    for (const m of line.matchAll(EMAIL)) if (!RESERVED_DOMAIN.test(m[1])) hit('email', m[0]);
    for (const m of line.matchAll(IPV4)) {
      const nums = m.slice(1, 5).map(Number);
      // A version string ("version 4.12.5.7", "v1.2.3.4") is not an address.
      const before = line.slice(Math.max(0, m.index - 12), m.index).toLowerCase();
      if (/(version|ver\.?|build|v)\s*[:=]?\s*$/.test(before)) continue;
      if (!isHarmlessIp(nums)) hit('public-ip', m[0]);
    }
    for (const m of line.matchAll(INTERNAL_SUFFIX)) if (!RESERVED_DOMAIN.test(m[0])) hit('internal-host', m[0]);
  });
  return out;
}

function* walk(dir) {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    if (SKIP_DIRS.has(e.name)) continue;
    const p = join(dir, e.name);
    if (e.isDirectory()) yield* walk(p);
    else if (e.isFile() && !BINARY_EXT.test(e.name)) yield p;
  }
}

export function defaultRoots(repo = REPO) {
  const roots = [join(repo, 'samples')];
  const crates = join(repo, 'crates');
  if (existsSync(crates)) {
    for (const c of readdirSync(crates)) {
      const f = join(crates, c, 'tests', 'fixtures');
      if (existsSync(f)) roots.push(f);
    }
  }
  return roots.filter(existsSync);
}

export function loadAllow(file = join(REPO, 'scripts/ci/scan-fixtures.allow.json')) {
  try {
    const v = JSON.parse(readFileSync(file, 'utf8'));
    return { values: Array.isArray(v.values) ? v.values.filter((x) => typeof x === 'string') : [] };
  } catch {
    return { values: [] };
  }
}

export function main(argv, log = console) {
  const roots = argv.length ? argv.map((a) => resolve(a)) : defaultRoots();
  const allow = loadAllow();
  let bad = 0;
  let files = 0;
  for (const root of roots) {
    for (const file of walk(root)) {
      let st;
      try {
        st = statSync(file);
      } catch {
        continue;
      }
      if (st.size > MAX_BYTES) continue;
      const buf = readFileSync(file);
      if (buf.subarray(0, 4096).includes(0)) continue; // binary (UTF-16 text is covered by the .bin/.dmp list and review)
      files++;
      for (const f of scanText(buf.toString('utf8'), allow)) {
        log.error(`${relative(REPO, file)}:${f.line}: ${f.kind} ${f.text}`);
        bad++;
      }
    }
  }
  log.log(`scanned ${files} text file(s) in ${roots.length} folder(s): ${bad} finding(s)`);
  if (bad) log.error('Replace the value with a documentation-range one (192.0.2.x, user@example.com, host.example.test), or, for a reviewed false positive, add it to scripts/ci/scan-fixtures.allow.json.');
  return bad ? 1 : 0;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) process.exit(main(process.argv.slice(2)));
