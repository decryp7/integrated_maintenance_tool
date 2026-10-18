// Pure helpers for IMT's translation files: XLIFF 1.2 (what `ng extract-i18n` writes and translators edit) to the
// runtime JSON that `loadTranslations` takes, and the generated pseudo-locale. No I/O here; see build-locales.mjs.

const ENTITIES = { lt: '<', gt: '>', quot: '"', apos: "'", amp: '&' };

function unescapeXml(s) {
  return s.replace(/&(lt|gt|quot|apos|amp|#(\d+)|#x([0-9a-fA-F]+));/g, (_, name, dec, hex) =>
    dec ? String.fromCodePoint(Number(dec)) : hex ? String.fromCodePoint(parseInt(hex, 16)) : ENTITIES[name],
  );
}

/** XLIFF message content -> runtime text: `<x id="PH"/>` becomes `{$PH}`, entities are decoded. */
export function xliffContentToText(content) {
  return unescapeXml(content.replace(/<x\s+id="([^"]+)"[^>]*\/>/g, (_, id) => `{$${id}}`));
}

/** XLIFF content in the exact runtime form (placeholders inside ICU cases are written without the `$`). */
function runtimeText(content) {
  return serializeMessage(parseMessage(xliffContentToText(content)));
}

function noteOf(body, from) {
  const m = new RegExp(`<note[^>]*from="${from}"[^>]*>([\\s\\S]*?)</note>`).exec(body);
  return m ? unescapeXml(m[1]) : undefined;
}

/**
 * Parses an XLIFF 1.2 file into messages: `{ id, source, target?, meaning?, description? }` in runtime text form.
 * Throws on a file without trans-units or with a unit that has no source (a broken file must not pass silently).
 */
export function parseXliff(xml) {
  const units = [...xml.matchAll(/<trans-unit\s+id="([^"]+)"[^>]*>([\s\S]*?)<\/trans-unit>/g)];
  return units.map(([, id, body]) => {
    const source = /<source>([\s\S]*?)<\/source>/.exec(body);
    if (!source) throw new Error(`trans-unit ${id} has no <source>`);
    const target = /<target(?:\s[^>]*)?>([\s\S]*?)<\/target>/.exec(body);
    const msg = { id: unescapeXml(id), source: runtimeText(source[1]) };
    if (target) msg.target = runtimeText(target[1]);
    const meaning = noteOf(body, 'meaning');
    const description = noteOf(body, 'description');
    if (meaning !== undefined) msg.meaning = meaning;
    if (description !== undefined) msg.description = description;
    return msg;
  });
}

/** What must stay in sync between the committed source file and a fresh extraction (locations are ignored). */
export function normalizeMessages(messages) {
  return messages
    .map(({ id, source, meaning, description }) => ({ id, source, meaning: meaning ?? null, description: description ?? null }))
    .sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
}

// ---- runtime message syntax: text, {$PLACEHOLDER} and ICU {VAR, plural|select, case {message} ...} ----

/** An ICU expression as Angular extracts it from a template: `{VAR_PLURAL, plural, ...}`, `{VAR_SELECT_1, select, ...}`. */
const ICU_START = /^\{VAR_(?:PLURAL|SELECT)(?:_\d+)?\s*,/;
/** A placeholder inside an ICU case: `{INTERPOLATION}`, `{START_TAG_B}` (no `$`). */
const ICU_PLACEHOLDER = /^\{[A-Z][A-Z0-9_]*\}/;

/**
 * Parses runtime message text into nodes: `{ text }`, `{ ph }` and `{ icu: { name, type, cases: [{ key, nodes }] } }`.
 * Any other brace is literal text (`$localize` messages from TypeScript have no ICU).
 */
export function parseMessage(s) {
  let i = 0;
  function nodes(inCase) {
    const out = [];
    let text = '';
    const flush = () => {
      if (text) out.push({ text });
      text = '';
    };
    while (i < s.length) {
      const c = s[i];
      if (c === '{' && s[i + 1] === '$') {
        const end = s.indexOf('}', i);
        if (end < 0) throw new Error(`unclosed placeholder in: ${s}`);
        flush();
        out.push({ ph: s.slice(i + 2, end) });
        i = end + 1;
      } else if (c === '{' && inCase && ICU_PLACEHOLDER.test(s.slice(i))) {
        const end = s.indexOf('}', i);
        flush();
        out.push({ ph: s.slice(i + 1, end) });
        i = end + 1;
      } else if (c === '{' && ICU_START.test(s.slice(i, i + 40))) {
        flush();
        out.push({ icu: icu() });
      } else if (c === '}' && inCase) {
        flush();
        i++;
        return out;
      } else {
        text += c;
        i++;
      }
    }
    if (inCase) throw new Error(`unclosed ICU case in: ${s}`);
    flush();
    return out;
  }
  function until(ch) {
    const end = s.indexOf(ch, i);
    if (end < 0) throw new Error(`expected "${ch}" in: ${s}`);
    const v = s.slice(i, end).trim();
    i = end + 1;
    return v;
  }
  function icu() {
    i++; // {
    const name = until(',');
    const type = until(',');
    const cases = [];
    for (;;) {
      while (/\s/.test(s[i] ?? '')) i++;
      if (s[i] === '}') {
        i++;
        return { name, type, cases };
      }
      if (i >= s.length) throw new Error(`unclosed ICU in: ${s}`);
      const key = until('{');
      cases.push({ key, nodes: nodes(true) });
    }
  }
  return nodes(false);
}

/**
 * Inverse of `parseMessage`. Placeholders are written `{$NAME}` in a message but `{NAME}` inside ICU cases, which is
 * how Angular's runtime (and `ng extract-i18n --format json`) expects them.
 */
export function serializeMessage(nodes, inIcu = false) {
  return nodes
    .map((n) => {
      if ('text' in n) return n.text;
      if ('ph' in n) return inIcu ? `{${n.ph}}` : `{$${n.ph}}`;
      const cases = n.icu.cases.map((c) => `${c.key} {${serializeMessage(c.nodes, true)}}`).join(' ');
      return `{${n.icu.name}, ${n.icu.type}, ${cases}}`;
    })
    .join('');
}

/** Placeholder names a message uses (incl. inside ICU cases), sorted, for comparing a translation with its source. */
export function placeholders(nodes, acc = new Set()) {
  for (const n of nodes) {
    if ('ph' in n) acc.add(n.ph);
    else if ('icu' in n) for (const c of n.icu.cases) placeholders(c.nodes, acc);
  }
  return [...acc].sort();
}

// ---- pseudo-locale ----

const ACCENTS = {
  a: 'á', b: 'ƀ', c: 'ç', d: 'ð', e: 'é', f: 'ƒ', g: 'ĝ', h: 'ĥ', i: 'î', j: 'ĵ', k: 'ķ', l: 'ļ', m: 'ɱ',
  n: 'ñ', o: 'ö', p: 'þ', q: 'ǫ', r: 'ŕ', s: 'š', t: 'ţ', u: 'û', v: 'ṽ', w: 'ŵ', x: 'ẋ', y: 'ý', z: 'ž',
  A: 'Å', B: 'Ɓ', C: 'Ç', D: 'Ð', E: 'É', F: 'Ƒ', G: 'Ĝ', H: 'Ĥ', I: 'Î', J: 'Ĵ', K: 'Ķ', L: 'Ļ', M: 'Ṁ',
  N: 'Ñ', O: 'Ö', P: 'Þ', Q: 'Ǫ', R: 'Ŕ', S: 'Š', T: 'Ţ', U: 'Û', V: 'Ṽ', W: 'Ŵ', X: 'Ẋ', Y: 'Ý', Z: 'Ž',
};
const VOWELS = new Set('aeiouAEIOU');

/**
 * Accents every ASCII letter and doubles three of every four vowels (about 30% longer, spread through the words so
 * wrapping stays realistic).
 */
export function pseudoText(text) {
  let vowel = 0;
  let out = '';
  for (const ch of text) {
    const a = ACCENTS[ch] ?? ch;
    out += a;
    if (VOWELS.has(ch) && vowel++ % 4 !== 3) out += a;
  }
  return out;
}

function pseudoNodes(nodes) {
  return nodes.map((n) => {
    if ('text' in n) return { text: pseudoText(n.text) };
    if ('icu' in n) return { icu: { ...n.icu, cases: n.icu.cases.map((c) => ({ key: c.key, nodes: bracket(pseudoNodes(c.nodes)) })) } };
    return n;
  });
}

function bracket(nodes) {
  return [{ text: '⟦' }, ...nodes, { text: '⟧' }];
}

/**
 * The pseudo translation of a source message: accented, longer and bracketed (⟦…⟧) so truncation and unmarked
 * strings stand out. Placeholders and ICU syntax are kept; a message that is a single ICU is bracketed per case.
 */
export function pseudoMessage(source) {
  const nodes = parseMessage(source);
  const pseudo = pseudoNodes(nodes);
  const singleIcu = nodes.length === 1 && 'icu' in nodes[0];
  return serializeMessage(singleIcu ? pseudo : bracket(pseudo));
}

/** Runtime translations of the pseudo-locale for every source message. */
export function pseudoTranslations(messages) {
  return Object.fromEntries(messages.map((m) => [m.id, pseudoMessage(m.source)]));
}

/**
 * Runtime translations from a translated XLIFF file. Units without a target are left out (they fall back to English
 * at run time); a target whose placeholders differ from the source is an error.
 */
export function targetTranslations(messages) {
  const out = {};
  for (const m of messages) {
    if (m.target === undefined || m.target.trim() === '') continue;
    const want = placeholders(parseMessage(m.source)).join(',');
    const got = placeholders(parseMessage(m.target)).join(',');
    if (want !== got) throw new Error(`${m.id}: the translation uses placeholders [${got}], the source [${want}]`);
    out[m.id] = m.target;
  }
  return out;
}

/** The JSON file `loadTranslations` is fed with (same shape as `ng extract-i18n --format json`). */
export function runtimeJson(locale, translations) {
  const sorted = Object.fromEntries(Object.entries(translations).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));
  return JSON.stringify({ locale, translations: sorted }, null, 2) + '\n';
}
