import { loadTranslations } from '@angular/localize';
import { LanguageStorage, LocalStorageLanguageStorage } from './language-storage';
import { language, PSEUDO_LOCALE, resolveLocale, setActiveLocale, SOURCE_LOCALE, textDirection } from './locale';

/** Message id -> translated text, in the `$localize` runtime format (`{$PLACEHOLDER}` placeholders). */
export type Translations = Record<string, string>;

/** Loads a locale's messages. The JSON is bundled as a lazy chunk: nothing is fetched from the network. */
export type TranslationLoader = () => Promise<Translations>;

/** Locales with a translation file (the source locale needs none). */
export const TRANSLATION_LOADERS: Readonly<Record<string, TranslationLoader>> = {
  [PSEUDO_LOCALE]: () => import('../../locale/messages.pseudo.json').then((m) => m.default.translations as Translations),
};

export interface I18nDeps {
  storage: LanguageStorage;
  languages: readonly string[];
  loaders: Readonly<Record<string, TranslationLoader>>;
  load: (translations: Translations) => void;
  root: HTMLElement;
}

/** The exact locale's messages, else another region's of the same language (never the pseudo-locale's). */
export function loaderFor(locale: string, loaders: Readonly<Record<string, TranslationLoader>>): TranslationLoader | undefined {
  if (loaders[locale]) return loaders[locale];
  if (locale === PSEUDO_LOCALE) return undefined;
  const key = Object.keys(loaders).find((k) => k !== PSEUDO_LOCALE && language(k) === language(locale));
  return key ? loaders[key] : undefined;
}

function defaultDeps(): I18nDeps {
  return {
    storage: new LocalStorageLanguageStorage(),
    languages: navigator.languages?.length ? navigator.languages : [navigator.language],
    loaders: TRANSLATION_LOADERS,
    load: loadTranslations,
    root: document.documentElement,
  };
}

/**
 * Picks the locale from the stored choice and the system languages, loads its translations and sets `lang`/`dir`
 * on the document. Must run before any module that uses `$localize` is imported. A missing or broken translation
 * file falls back to English instead of failing the start. Returns the locale the app runs in.
 */
export async function initI18n(deps: Partial<I18nDeps> = {}): Promise<string> {
  const d = { ...defaultDeps(), ...deps };
  let locale = resolveLocale(d.storage.load(), d.languages);
  const loader = loaderFor(locale, d.loaders);
  if (loader) {
    try {
      d.load(await loader());
    } catch (err) {
      console.warn(`Debrief: could not load the ${locale} translations; using English.`, err);
      locale = SOURCE_LOCALE;
    }
  }
  setActiveLocale(locale);
  d.root.lang = locale;
  d.root.dir = textDirection(locale);
  return locale;
}
