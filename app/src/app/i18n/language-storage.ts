import { LANGUAGE_CHOICES, LanguageChoice } from './locale';

/**
 * Where the language choice is persisted. Kept apart from the other settings so `main.ts` can read it before
 * bootstrap without loading Angular. Implementations must never throw.
 */
export interface LanguageStorage {
  load(): LanguageChoice;
  save(choice: LanguageChoice): void;
}

export const LANGUAGE_KEY = 'imt.language';
export const LANGUAGE_VERSION = 1;

/** Validates an untrusted stored value; anything unknown is "System default". */
export function parseLanguage(raw: unknown): LanguageChoice {
  return LANGUAGE_CHOICES.includes(raw as LanguageChoice) ? (raw as LanguageChoice) : 'system';
}

export class LocalStorageLanguageStorage implements LanguageStorage {
  constructor(private readonly key = LANGUAGE_KEY) {}

  load(): LanguageChoice {
    try {
      const parsed = JSON.parse(localStorage.getItem(this.key) ?? 'null') as { version?: number; language?: unknown } | null;
      return parsed?.version === LANGUAGE_VERSION ? parseLanguage(parsed.language) : 'system';
    } catch {
      return 'system';
    }
  }

  save(choice: LanguageChoice): void {
    try {
      localStorage.setItem(this.key, JSON.stringify({ version: LANGUAGE_VERSION, language: choice }));
    } catch {
      /* storage unavailable: the language stays "System default" */
    }
  }
}
