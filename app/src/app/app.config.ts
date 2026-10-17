import { ApplicationConfig, provideBrowserGlobalErrorListeners } from '@angular/core';
import { provideBackend } from './core/backend';
import { provideErrorLogging, provideLogger } from './core/logger';

export const appConfig: ApplicationConfig = {
  providers: [provideBrowserGlobalErrorListeners(), provideLogger(), provideErrorLogging(), provideBackend()],
};
