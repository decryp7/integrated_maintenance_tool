import { inject, Injectable, InjectionToken, signal } from '@angular/core';
import { DateFormat } from '../core/format';

export type GridDensity = 'compact' | 'comfortable';

/** What the coloured source marker on log lines stands for: the file, the instance (timeline lane), or nothing. */
export type SourceColorMode = 'source' | 'instance' | 'off';

/** Settings kept here (theme, time zone and the "always show start/stop" default live in `WorkspaceStore`). */
export interface Settings {
  dateFormat: DateFormat;
  density: GridDensity;
  sourceColors: SourceColorMode;
}

export const DEFAULT_SETTINGS: Settings = { dateFormat: 'readable', density: 'comfortable', sourceColors: 'source' };

/** Where settings are persisted. Implementations must never throw. */
export interface SettingsStorage {
  load(): Partial<Settings>;
  save(settings: Settings): void;
}

export const SETTINGS_KEY = 'debrief.settings';
export const SETTINGS_VERSION = 1;

export class LocalStorageSettingsStorage implements SettingsStorage {
  constructor(private readonly key = SETTINGS_KEY) {}

  load(): Partial<Settings> {
    try {
      const parsed = JSON.parse(localStorage.getItem(this.key) ?? 'null') as { version?: number; settings?: Partial<Settings> } | null;
      return parsed?.version === SETTINGS_VERSION && parsed.settings && typeof parsed.settings === 'object' ? parsed.settings : {};
    } catch {
      return {};
    }
  }

  save(settings: Settings): void {
    try {
      localStorage.setItem(this.key, JSON.stringify({ version: SETTINGS_VERSION, settings }));
    } catch {
      /* storage unavailable: settings are a convenience only */
    }
  }
}

export const SETTINGS_STORAGE = new InjectionToken<SettingsStorage>('SETTINGS_STORAGE', {
  providedIn: 'root',
  factory: () => new LocalStorageSettingsStorage(),
});

const DATE_FORMATS: DateFormat[] = ['readable', 'iso'];
const DENSITIES: GridDensity[] = ['compact', 'comfortable'];
export const SOURCE_COLOR_MODES: readonly SourceColorMode[] = ['source', 'instance', 'off'];

/** Validates untrusted stored values; anything unknown falls back to the default. */
export function sanitizeSettings(raw: Partial<Settings>): Settings {
  return {
    dateFormat: DATE_FORMATS.includes(raw.dateFormat as DateFormat) ? (raw.dateFormat as DateFormat) : DEFAULT_SETTINGS.dateFormat,
    density: DENSITIES.includes(raw.density as GridDensity) ? (raw.density as GridDensity) : DEFAULT_SETTINGS.density,
    sourceColors: SOURCE_COLOR_MODES.includes(raw.sourceColors as SourceColorMode) ? (raw.sourceColors as SourceColorMode) : DEFAULT_SETTINGS.sourceColors,
  };
}

/** Display preferences that are not part of the workspace state. */
@Injectable({ providedIn: 'root' })
export class SettingsStore {
  private readonly storage = inject(SETTINGS_STORAGE);
  private readonly initial = sanitizeSettings(this.storage.load());

  readonly dateFormat = signal<DateFormat>(this.initial.dateFormat);
  readonly density = signal<GridDensity>(this.initial.density);
  readonly sourceColors = signal<SourceColorMode>(this.initial.sourceColors);

  setDateFormat(dateFormat: DateFormat): void {
    this.dateFormat.set(dateFormat);
    this.persist();
  }

  setDensity(density: GridDensity): void {
    this.density.set(density);
    this.persist();
  }

  setSourceColors(mode: SourceColorMode): void {
    this.sourceColors.set(mode);
    this.persist();
  }

  private persist(): void {
    this.storage.save({ dateFormat: this.dateFormat(), density: this.density(), sourceColors: this.sourceColors() });
  }
}
