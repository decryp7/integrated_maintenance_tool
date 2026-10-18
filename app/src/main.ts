/// <reference types="@angular/localize" />
import { initI18n } from './app/i18n/boot';

// Translations are loaded at run time (one build serves every language), so they must be in place before any
// module that evaluates `$localize` is imported: the app is loaded only after `initI18n`.
initI18n()
  .then(() => import('./bootstrap'))
  .catch((err) => console.error(err));
