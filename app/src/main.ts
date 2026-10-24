/// <reference types="@angular/localize" />
import { initI18n } from './app/i18n/boot';
import { reportBootstrapFailure, startBootErrorForwarding, tauriBootErrorSender } from './app/core/startup/boot-errors';

// Errors before the window is ready reach the Rust log (ADR 0058): a window that never starts leaves its cause there.
if ('__TAURI_INTERNALS__' in window) startBootErrorForwarding(tauriBootErrorSender);

// Translations are loaded at run time (one build serves every language), so they must be in place before any
// module that evaluates `$localize` is imported: the app is loaded only after `initI18n`.
initI18n()
  .then(() => import('./bootstrap'))
  .catch((err: unknown) => {
    console.error(err);
    reportBootstrapFailure(err);
  });
