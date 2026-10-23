/**
 * The UI language and the locale every formatter uses. Pure: no Angular, no `$localize`, so it can run in `main.ts`
 * before the application (and any translated module) is loaded.
 */

/** What the user picked in Settings ▸ General ▸ Language. */
export type LanguageChoice = 'system' | 'en' | 'pseudo';

export const LANGUAGE_CHOICES: readonly LanguageChoice[] = ['system', 'en', 'pseudo'];

/** The language the source strings are written in (no translations are loaded for it). */
export const SOURCE_LOCALE = 'en-US';

/** The generated pseudo-locale (accented, longer, bracketed text) used to find unmarked strings and overflow. */
export const PSEUDO_LOCALE = 'en-XA';

/** Real translations Debrief ships, by BCP 47 tag. "System default" picks one of these or falls back to English. */
export const SHIPPED_LOCALES: readonly string[] = [SOURCE_LOCALE];

const RTL_LANGUAGES = new Set(['ar', 'arc', 'ckb', 'dv', 'fa', 'he', 'ks', 'ku', 'ps', 'sd', 'syr', 'ug', 'ur', 'yi']);

/** The language subtag ("en" for "en-GB"), lower-case. */
export function language(tag: string): string {
  return tag.trim().toLowerCase().split(/[-_]/)[0] ?? '';
}

/**
 * The locale to run in. "System default" uses the first preferred system language Debrief has messages for (keeping the
 * user's region, e.g. en-GB, for formatting) and otherwise English.
 */
export function resolveLocale(choice: LanguageChoice, preferred: readonly string[]): string {
  if (choice === 'en') return SOURCE_LOCALE;
  if (choice === 'pseudo') return PSEUDO_LOCALE;
  for (const tag of preferred) {
    if (!tag || SHIPPED_LOCALES.every((l) => language(l) !== language(tag))) continue;
    try {
      return Intl.getCanonicalLocales(tag)[0] ?? SOURCE_LOCALE;
    } catch {
      /* not a valid tag: try the next one */
    }
  }
  return SOURCE_LOCALE;
}

/** Text direction for a locale. */
export function textDirection(locale: string): 'ltr' | 'rtl' {
  return RTL_LANGUAGES.has(language(locale)) ? 'rtl' : 'ltr';
}

let active = SOURCE_LOCALE;

/** The locale the app runs in (set once at startup, before bootstrap). */
export function activeLocale(): string {
  return active;
}

/** Sets the running locale. Called by `initI18n`; tests may call it and must restore `SOURCE_LOCALE`. */
export function setActiveLocale(locale: string): void {
  active = locale;
  numberFormat = null;
  decimal = null;
  pluralRules = null;
}

let numberFormat: Intl.NumberFormat | null = null;
let decimal: string | null = null;
let pluralRules: Intl.PluralRules | null = null;

/** Integers with the active locale's grouping ("12,345" in English). */
export function groupedNumber(n: number): string {
  numberFormat ??= new Intl.NumberFormat(active);
  return numberFormat.format(n);
}

/** Swaps the "." of an already rounded decimal string ("1.5") for the active locale's separator. */
export function localizeDecimal(value: string | number): string {
  const s = String(value);
  decimal ??= new Intl.NumberFormat(active).formatToParts(1.5).find((p) => p.type === 'decimal')?.value ?? '.';
  return decimal === '.' ? s : s.replace('.', decimal);
}

/** The plural category of `n` in the active locale ("one", "other", ...). */
export function pluralCategory(n: number): Intl.LDMLPluralRule {
  pluralRules ??= new Intl.PluralRules(active);
  return pluralRules.select(n);
}

/** Plural forms of one message; `other` is required, `=0` (if given) wins for zero. */
export type PluralForms = { other: string; '=0'?: string } & Partial<Record<Intl.LDMLPluralRule, string>>;

/**
 * Chooses the form for `n`: the TypeScript counterpart of an ICU plural in a template (`$localize` has no ICU).
 * Each form is its own `$localize` message, e.g.
 * `plural(n, { one: $localize`:@@x.spans.one:${n}:count: span`, other: $localize`:@@x.spans.other:${n}:count: spans` })`.
 */
export function plural(n: number, forms: PluralForms): string {
  if (n === 0 && forms['=0'] !== undefined) return forms['=0'];
  return forms[pluralCategory(n)] ?? forms.other;
}
