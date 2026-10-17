import { ApplicationConfig, provideBrowserGlobalErrorListeners } from '@angular/core';
import { provideImtBuiltins } from './builtin/provide-builtins';
import { provideBackend } from './core/backend';
import { provideErrorLogging, provideLogger } from './core/logger';
import { provideImtPlugin } from './extensions';
import { provideDiagnostics } from './diagnostics/provide-diagnostics';
import { provideAnalysis } from './analysis/analysis.plugin';
import { provideTools } from './tools/provide-tools';
import { provideMarkers } from './markers/markers.plugin';
import { REGISTRY_PLUGIN } from './plugins/registry/registry.plugin';

export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    provideLogger(),
    provideErrorLogging(),
    provideDiagnostics(),
    provideBackend(),
    provideImtBuiltins(),
    provideImtPlugin(REGISTRY_PLUGIN),
    provideMarkers(),
    provideTools(),
    provideAnalysis(),
  ],
};
