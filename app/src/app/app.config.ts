import { ApplicationConfig, provideBrowserGlobalErrorListeners } from '@angular/core';
import { provideImtBuiltins } from './builtin/provide-builtins';
import { provideBackend } from './core/backend';
import { provideErrorLogging, provideLogger } from './core/logger';
import { provideImtPlugin } from './extensions';
import { REGISTRY_PLUGIN } from './plugins/registry/registry.plugin';

export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    provideLogger(),
    provideErrorLogging(),
    provideBackend(),
    provideImtBuiltins(),
    provideImtPlugin(REGISTRY_PLUGIN),
  ],
};
