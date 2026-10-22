import { HandOff, LeakKind, SearchEngine, SEARCH_ENGINES } from './browser.types';

/** The engines offered in pickers, in order. */
export const ENGINE_OPTIONS: readonly SearchEngine[] = SEARCH_ENGINES;

export function engineLabel(e: SearchEngine): string {
  switch (e) {
    case 'duckDuckGo':
      return 'DuckDuckGo';
    case 'google':
      return 'Google';
    case 'bing':
      return 'Bing';
    case 'custom':
      return $localize`:Search engine option@@browser.engine.custom:Custom`;
  }
}

/** What a highlighted part of an approval card is, for screen readers and tooltips. */
export function leakText(kind: LeakKind): string {
  switch (kind) {
    case 'logValue':
      return $localize`:@@browser.leak.logValue:from your logs`;
    case 'logText':
      return $localize`:@@browser.leak.logText:text from your logs`;
    case 'private':
      return $localize`:@@browser.leak.private:looks private`;
  }
}

export function handOffText(h: HandOff | null): string {
  switch (h) {
    case 'captcha':
      return $localize`:@@browser.handOff.captcha:The page shows a CAPTCHA or human check. The AI stopped; solve it yourself if you want to go on.`;
    case 'login':
      return $localize`:@@browser.handOff.login:The page asks to sign in. The AI stopped; sign in yourself if you want to go on.`;
    case 'payment':
      return $localize`:@@browser.handOff.payment:The page is a payment form. The AI stopped and never fills in payments.`;
    default:
      return $localize`:@@browser.handOff.other:The AI stopped and handed the browser back to you.`;
  }
}
