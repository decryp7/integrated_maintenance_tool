import { reportBackendFailures } from '../core/failure-reporter';
import { invoke } from '@tauri-apps/api/core';
import { inject, Injectable } from '@angular/core';
import { Logger, withFailureLogging } from '../core/logger';
import { isTauri } from '../core/platform';
import { MockSettingsBackend } from './mock-settings-backend';
import { ImportResult, SetResult, SettingsOverview, SettingValue } from './settings.types';

/**
 * What the Settings screen asks the engine. Validation, defaults and persistence happen in Rust;
 * the UI only renders the schema and sends values. Injected as a class token: the Tauri backend
 * in the app, the in-memory mock in a plain browser (dev server, e2e tests).
 */
@Injectable({
  providedIn: 'root',
  useFactory: () =>
    withFailureLogging<SettingsBackend>(isTauri() ? new TauriSettingsBackend() : new MockSettingsBackend(), inject(Logger), {
      onFailure: reportBackendFailures(),
    }),
})
export abstract class SettingsBackend {
  /** Every section with its current values and status. */
  abstract sections(): Promise<SettingsOverview>;
  /** Validates, applies and saves a section's values (`null` = default). Invalid values come back as `errors`. */
  abstract set(section: string, values: Record<string, SettingValue | null>): Promise<SetResult>;
  /** Puts a whole section back to its defaults. */
  abstract reset(section: string): Promise<SetResult>;
  /** Writes all settings to a user-chosen `.json` file. */
  abstract exportTo(path: string): Promise<void>;
  /** Reads a settings file and applies each valid section. */
  abstract importFrom(path: string): Promise<ImportResult>;
}

/** Talks to `app/src-tauri/src/settings.rs`. */
export class TauriSettingsBackend extends SettingsBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }
  sections() {
    return this.call<SettingsOverview>('settings_sections');
  }
  set(section: string, values: Record<string, SettingValue | null>) {
    return this.call<SetResult>('set_settings', { section, values });
  }
  reset(section: string) {
    return this.call<SetResult>('reset_settings', { section });
  }
  exportTo(path: string) {
    return this.call<void>('export_settings', { path });
  }
  importFrom(path: string) {
    return this.call<ImportResult>('import_settings', { path });
  }
}
