#!/usr/bin/env node
// Runs `ng <command> …` with the build's commit and date defined as DEBRIEF_BUILD_INFO (read by
// src/app/about/build-info.ts and shown in About Debrief). Never fails the build for missing git:
// the values fall back to 'unknown'.
//
// Usage: node scripts/build-info.mjs build [ng args…]   (npm run build)
//        node scripts/build-info.mjs serve [ng args…]   (npm start)
import { execFileSync, spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));

/** `git rev-parse --short HEAD` in the app directory, or null when git or the repository is unavailable. */
export function gitShortHead(cwd = here) {
  try {
    const out = execFileSync('git', ['rev-parse', '--short', 'HEAD'], { cwd, stdio: ['ignore', 'pipe', 'ignore'], timeout: 5000 });
    return out.toString().trim() || null;
  } catch {
    return null;
  }
}

/** The build info: short commit (or 'unknown') and the UTC build date (YYYY-MM-DD). */
export function buildInfo({ head = gitShortHead, now = () => new Date() } = {}) {
  const commit = head();
  return { commit: commit && /^[0-9a-f]{4,40}$/.test(commit) ? commit : 'unknown', date: now().toISOString().slice(0, 10) };
}

/** The `--define` value for the Angular builder (a JS object literal). */
export function defineArg(info) {
  return `DEBRIEF_BUILD_INFO=${JSON.stringify({ commit: info.commit, date: info.date })}`;
}

/** Arguments for `node <ng.js> …`: the command, its arguments and the define. */
export function ngArgs(argv, info) {
  return [...argv, '--define', defineArg(info)];
}

/* c8 ignore start: process entry point */
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const argv = process.argv.slice(2);
  if (!argv.length) {
    console.error('usage: node scripts/build-info.mjs <ng command> [args…]');
    process.exit(2);
  }
  const info = buildInfo();
  console.error(`build info: commit ${info.commit}, date ${info.date}`);
  // Run the Angular CLI through node directly (no shell), so it behaves the same on Windows.
  const ng = createRequire(import.meta.url).resolve('@angular/cli/bin/ng.js');
  const r = spawnSync(process.execPath, [ng, ...ngArgs(argv, info)], { stdio: 'inherit', cwd: resolve(here, '..') });
  process.exit(r.status ?? 1);
}
/* c8 ignore stop */
