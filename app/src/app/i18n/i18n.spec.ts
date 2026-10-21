import { TestBed } from '@angular/core/testing';
import { DialogService } from '../ui/dialog.service';
import { initI18n, loaderFor, TRANSLATION_LOADERS } from './boot';
import { LANGUAGE_STORAGE, LanguageService, PAGE_RELOAD, SYSTEM_LANGUAGES } from './language.service';
import { LANGUAGE_KEY, LanguageStorage, LocalStorageLanguageStorage, parseLanguage } from './language-storage';
import {
  activeLocale,
  groupedNumber,
  LanguageChoice,
  localizeDecimal,
  plural,
  pluralCategory,
  PSEUDO_LOCALE,
  resolveLocale,
  setActiveLocale,
  SOURCE_LOCALE,
  textDirection,
} from './locale';
import { formatCompact, formatCount, formatDateTime, formatDuration, formatNumber } from '../core/format';

class MemoryStorage implements LanguageStorage {
  constructor(public value: LanguageChoice = 'system') {}
  load(): LanguageChoice {
    return this.value;
  }
  save(choice: LanguageChoice): void {
    this.value = choice;
  }
}

afterEach(() => setActiveLocale(SOURCE_LOCALE));

describe('locale', () => {
  it('resolves the choice: explicit English and pseudo, system languages IMT ships, else English', () => {
    expect(resolveLocale('en', ['de-DE'])).toBe(SOURCE_LOCALE);
    expect(resolveLocale('pseudo', ['en-US'])).toBe(PSEUDO_LOCALE);
    expect(resolveLocale('system', ['de-DE', 'fr'])).toBe(SOURCE_LOCALE);
    expect(resolveLocale('system', ['de-DE', 'en-gb'])).toBe('en-GB');
    expect(resolveLocale('system', ['', 'en-US'])).toBe('en-US');
    expect(resolveLocale('system', ['en-not a tag', 'en'])).toBe('en');
    expect(resolveLocale('system', [])).toBe(SOURCE_LOCALE);
  });

  it('knows the text direction', () => {
    expect(textDirection('ar-EG')).toBe('rtl');
    expect(textDirection('he')).toBe('rtl');
    expect(textDirection('en-US')).toBe('ltr');
    expect(textDirection(PSEUDO_LOCALE)).toBe('ltr');
  });

  it('formats numbers and plurals with the active locale', () => {
    expect(groupedNumber(1234567)).toBe('1,234,567');
    expect(localizeDecimal('1.5')).toBe('1.5');
    expect(pluralCategory(1)).toBe('one');
    expect(plural(1, { one: 'one span', other: 'spans' })).toBe('one span');
    expect(plural(2, { one: 'one span', other: 'spans' })).toBe('spans');
    expect(plural(0, { '=0': 'none', other: 'spans' })).toBe('none');
    expect(plural(0, { other: 'spans' })).toBe('spans');

    setActiveLocale('de-DE');
    expect(activeLocale()).toBe('de-DE');
    expect(groupedNumber(1234567)).toBe('1.234.567');
    expect(localizeDecimal('1.5')).toBe('1,5');
    expect(formatCount(12345)).toBe('12.345');
    expect(formatCompact(1234)).toBe('1,2k');
    expect(formatNumber(2.25)).toBe('2,25');
    expect(formatDuration(1500)).toBe('1,5 s');
    expect(formatDateTime(Date.UTC(2026, 9, 14, 10, 41, 22), 'utc')).toBe('14 Okt 10:41:22');
  });

  it('keeps the English output of the format helpers', () => {
    expect(formatCount(12345)).toBe('12,345');
    expect(formatCompact(2_500_000)).toBe('2.5M');
    expect(formatNumber(12345)).toBe('12k');
    expect(formatDuration(90_000)).toBe('1 min 30 s');
    expect(formatDateTime(Date.UTC(2026, 9, 14, 10, 41, 22), 'utc')).toBe('14 Oct 10:41:22');
  });
});

describe('language storage', () => {
  beforeEach(() => localStorage.clear());

  it('round-trips a choice and falls back to system default for anything else', () => {
    const s = new LocalStorageLanguageStorage();
    expect(s.load()).toBe('system');
    s.save('pseudo');
    expect(s.load()).toBe('pseudo');
    localStorage.setItem(LANGUAGE_KEY, JSON.stringify({ version: 9, language: 'en' }));
    expect(s.load()).toBe('system');
    localStorage.setItem(LANGUAGE_KEY, '{not json');
    expect(s.load()).toBe('system');
    expect(parseLanguage('klingon')).toBe('system');
    expect(parseLanguage('en')).toBe('en');
  });

  it('never throws when storage is unavailable', () => {
    const get = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    const set = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('denied');
    });
    const s = new LocalStorageLanguageStorage();
    expect(s.load()).toBe('system');
    expect(() => s.save('en')).not.toThrow();
    get.mockRestore();
    set.mockRestore();
  });
});

describe('initI18n', () => {
  it('loads the pseudo translations and sets lang and dir', async () => {
    const root = document.createElement('html');
    const load = vi.fn();
    const locale = await initI18n({
      storage: new MemoryStorage('pseudo'),
      languages: ['en-US'],
      loaders: { [PSEUDO_LOCALE]: async () => ({ 'a.b': 'Ţéšţ' }) },
      load,
      root,
    });
    expect(locale).toBe(PSEUDO_LOCALE);
    expect(load).toHaveBeenCalledWith({ 'a.b': 'Ţéšţ' });
    expect(activeLocale()).toBe(PSEUDO_LOCALE);
    expect(root.lang).toBe(PSEUDO_LOCALE);
    expect(root.dir).toBe('ltr');
  });

  it('loads nothing for English', async () => {
    const root = document.createElement('html');
    const load = vi.fn();
    expect(await initI18n({ storage: new MemoryStorage('system'), languages: ['de'], loaders: TRANSLATION_LOADERS, load, root })).toBe(SOURCE_LOCALE);
    expect(load).not.toHaveBeenCalled();
    expect(root.lang).toBe(SOURCE_LOCALE);
  });

  it('falls back to English when the translations cannot be loaded', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const root = document.createElement('html');
    const locale = await initI18n({
      storage: new MemoryStorage('pseudo'),
      languages: [],
      loaders: { [PSEUDO_LOCALE]: () => Promise.reject(new Error('corrupt')) },
      load: vi.fn(),
      root,
    });
    expect(locale).toBe(SOURCE_LOCALE);
    expect(root.lang).toBe(SOURCE_LOCALE);
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });

  it('uses the defaults (stored choice, navigator, bundled pseudo file)', async () => {
    localStorage.clear();
    expect(await initI18n()).toBe(SOURCE_LOCALE);
    expect(document.documentElement.lang).toBe(SOURCE_LOCALE);
  });

  it('bundles a pseudo translation file', async () => {
    const translations = await TRANSLATION_LOADERS[PSEUDO_LOCALE]!();
    expect(typeof translations).toBe('object');
  });

  it('finds the messages of the same language for another region, never the pseudo ones', () => {
    const fr = async () => ({});
    const pseudo = async () => ({});
    expect(loaderFor('fr-CA', { 'fr-FR': fr })).toBe(fr);
    expect(loaderFor('en-GB', { [PSEUDO_LOCALE]: pseudo })).toBeUndefined();
    expect(loaderFor(PSEUDO_LOCALE, { [PSEUDO_LOCALE]: pseudo })).toBe(pseudo);
    expect(loaderFor(PSEUDO_LOCALE, { 'en-US': fr })).toBeUndefined();
  });
});

describe('LanguageService', () => {
  function setup(initial: LanguageChoice, answer: boolean | undefined) {
    const storage = new MemoryStorage(initial);
    const reload = vi.fn();
    const open = vi.fn(() => ({ closed: Promise.resolve(answer), close: () => undefined }));
    TestBed.configureTestingModule({
      providers: [
        { provide: LANGUAGE_STORAGE, useValue: storage },
        { provide: PAGE_RELOAD, useValue: reload },
        { provide: SYSTEM_LANGUAGES, useValue: () => ['en-US'] },
        { provide: DialogService, useValue: { open } },
      ],
    });
    return { service: TestBed.inject(LanguageService), storage, reload, open };
  }

  it('saves a new language and restarts when the user confirms', async () => {
    const { service, storage, reload, open } = setup('system', true);
    await service.choose('pseudo');
    expect(storage.value).toBe('pseudo');
    expect(service.reloadPending()).toBe(true);
    expect(open).toHaveBeenCalledTimes(1);
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it('keeps the running language when the user says later', async () => {
    const { service, reload } = setup('system', undefined);
    await service.choose('pseudo');
    expect(reload).not.toHaveBeenCalled();
    expect(service.reloadPending()).toBe(true);
    service.reload();
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it('does not ask when the language the window runs in does not change', async () => {
    const { service, storage, open } = setup('system', true);
    await service.choose('system');
    await service.choose('en');
    expect(storage.value).toBe('en');
    expect(open).not.toHaveBeenCalled();
    expect(service.reloadPending()).toBe(false);
  });

  it('has working default providers', () => {
    TestBed.configureTestingModule({});
    expect(TestBed.inject(LANGUAGE_STORAGE)).toBeInstanceOf(LocalStorageLanguageStorage);
    expect(TestBed.inject(SYSTEM_LANGUAGES)().length).toBeGreaterThan(0);
    expect(typeof TestBed.inject(PAGE_RELOAD)).toBe('function');
  });
});
