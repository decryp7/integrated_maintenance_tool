import { readdirSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

/**
 * The units guard: quantities (sizes, durations, counts, rates, percentages) are formatted in one place
 * (`shared/format/units.ts`), never printed raw. A template interpolation whose last property looks like a quantity
 * (`file.sizeBytes`, `p.elapsedMs`, `s.total`) must go through a formatter call (`{{ size(f.sizeBytes) }}`). If a
 * value really is an identifier or an exact figure the user must see verbatim, add it to ALLOWED with the reason.
 */

const SRC = 'src/app';

/** Property names that hold a quantity. */
const QUANTITY = /(?:bytes|size|ms|millis|count|total|done|percent|pct|rate|duration|elapsed|offset|etaMs|etaSecs|events|lines|rows|files|items|samples|instances)$/i;

/** `file:expression` -> why printing it as is is right. */
const ALLOWED = new Map<string, string>([
  ['ai/proposals/parser-verdict.ts:v.zone.offset', 'a UTC offset the engine formats (e.g. +09:00), not a quantity'],
]);

interface Hit {
  file: string;
  expr: string;
}

function templates(): { file: string; text: string }[] {
  return readdirSync(SRC, { recursive: true })
    .map((p) => p.replaceAll('\\', '/'))
    .filter((p) => (p.endsWith('.ts') || p.endsWith('.html')) && !p.endsWith('.spec.ts') && !p.includes('/testing/') && !p.includes('/mock/') && !/mock-|test-support|\.types\.ts/.test(p))
    .map((file) => ({ file, text: readFileSync(`${SRC}/${file}`, 'utf8') }));
}

/** The raw interpolations: `{{ a.b.sizeBytes }}` with no call, pipe or operator around a quantity-named property. */
export function rawQuantities(file: string, text: string): Hit[] {
  const hits: Hit[] = [];
  for (const m of text.matchAll(/\{\{([^{}]*?)\}\}/g)) {
    const expr = (m[1] ?? '').replace(/\/\/.*$/, '').trim();
    if (!/^[A-Za-z_$][\w$]*(?:\??\.[A-Za-z_$][\w$]*|\(\))*\)?$/.test(expr) || expr.includes('(')) continue;
    const last = expr.split(/\??\./).pop() ?? '';
    if (QUANTITY.test(last)) hits.push({ file, expr });
  }
  return hits;
}

describe('units guard', () => {
  it('flags quantity-named interpolations that are not formatted', () => {
    expect(rawQuantities('x.ts', '{{ f.sizeBytes }} {{ size(f.sizeBytes) }} {{ a.b.elapsedMs }} {{ name }} {{ s.total }}').map((h) => h.expr)).toEqual([
      'f.sizeBytes',
      'a.b.elapsedMs',
      's.total',
    ]);
    expect(rawQuantities('x.ts', '{{ count(h.objectCount) // i18n(ph: "objects") }}')).toEqual([]);
  });

  it('prints no raw quantity in a template', () => {
    const raw = templates()
      .flatMap((t) => rawQuantities(t.file, t.text))
      .filter((h) => !ALLOWED.has(`${h.file}:${h.expr}`))
      .map((h) => `${h.file}: {{ ${h.expr} }}`);
    expect(raw, 'wrap these in a formatter from shared/format/units.ts, or allow-list them with a reason').toEqual([]);
  });

  it('keeps no stale allow-list entries', () => {
    const present = new Set(templates().flatMap((t) => rawQuantities(t.file, t.text).map((h) => `${h.file}:${h.expr}`)));
    expect([...ALLOWED.keys()].filter((k) => !present.has(k))).toEqual([]);
  });
});
