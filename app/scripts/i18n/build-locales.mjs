#!/usr/bin/env node
// Turns the extracted source messages (src/locale/messages.xlf, from `ng extract-i18n`) into the runtime JSON the app
// loads at startup:
//   - src/locale/messages.pseudo.json: the generated pseudo-locale (en-XA) for finding unmarked strings and overflow;
//   - src/locale/messages.<locale>.json for every translated src/locale/messages.<locale>.xlf (XLIFF with <target>s).
//
//   node scripts/i18n/build-locales.mjs                 regenerate the JSON files
//   node scripts/i18n/build-locales.mjs --check <xlf>   CI: fail unless messages.xlf matches the fresh extraction <xlf>
//                                                       (ids, texts, meanings, descriptions) and the JSON is current
import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { normalizeMessages, parseXliff, pseudoTranslations, runtimeJson, targetTranslations } from './locales.mjs';

const LOCALE_DIR = join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'src', 'locale');
const SOURCE = join(LOCALE_DIR, 'messages.xlf');
const PSEUDO_LOCALE = 'en-XA';

/** Every output file and its expected content. */
function outputs(sourceXml) {
  const files = new Map([[join(LOCALE_DIR, 'messages.pseudo.json'), runtimeJson(PSEUDO_LOCALE, pseudoTranslations(parseXliff(sourceXml)))]]);
  for (const name of readdirSync(LOCALE_DIR)) {
    const m = /^messages\.([A-Za-z]{2,3}(?:-[A-Za-z0-9]+)*)\.xlf$/.exec(name);
    if (!m) continue;
    const xml = readFileSync(join(LOCALE_DIR, name), 'utf8');
    files.set(join(LOCALE_DIR, `messages.${m[1]}.json`), runtimeJson(m[1], targetTranslations(parseXliff(xml))));
  }
  return files;
}

function fail(message) {
  console.error(`i18n: ${message}`);
  process.exit(1);
}

const args = process.argv.slice(2);
if (!existsSync(SOURCE)) fail(`${SOURCE} is missing: run \`npx ng extract-i18n\` first.`);
const sourceXml = readFileSync(SOURCE, 'utf8');

if (args[0] === '--check') {
  if (!args[1]) fail('--check needs the freshly extracted messages.xlf');
  const fresh = normalizeMessages(parseXliff(readFileSync(args[1], 'utf8')));
  const committed = normalizeMessages(parseXliff(sourceXml));
  if (JSON.stringify(fresh) !== JSON.stringify(committed)) {
    const ids = (list) => new Map(list.map((m) => [m.id, JSON.stringify(m)]));
    const [a, b] = [ids(committed), ids(fresh)];
    const changed = [...new Set([...a.keys(), ...b.keys()])].filter((id) => a.get(id) !== b.get(id));
    fail(`src/locale/messages.xlf is out of date (${changed.length} messages differ, e.g. ${changed.slice(0, 5).join(', ')}).\n` +
      'Run `npm run i18n` in app/ and commit the result.');
  }
  for (const [file, content] of outputs(sourceXml)) {
    if (!existsSync(file) || readFileSync(file, 'utf8').replace(/\r\n/g, '\n') !== content) fail(`${file} is out of date: run \`npm run i18n\` in app/.`);
  }
  console.log(`i18n: ${committed.length} messages, extraction and runtime files are up to date.`);
} else {
  for (const [file, content] of outputs(sourceXml)) writeFileSync(file, content);
  console.log(`i18n: wrote the runtime translations for ${parseXliff(sourceXml).length} messages.`);
}
