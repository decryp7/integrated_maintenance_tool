import { effect, inject, Injectable, signal, untracked } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { DockService } from '../dock/dock.service';
import { DebriefCommand } from '../extensions/tokens';
import { UiStateContribution } from '../lifecycle/ui-state';
import { defaultConfig, MAIN_TIMELINE, MAX_TIMELINES, nextTimelineId, sanitizeConfigs, TimelineConfig } from './timeline-config';

/** Where the settings of additional timelines are kept between runs (next to the dock layout, which holds the panels). */
export const TIMELINES_KEY = 'debrief.timelines';
const STORAGE_VERSION = 1;

/** Reads the stored configs; anything missing or corrupt counts as none. */
export function loadTimelines(key = TIMELINES_KEY): TimelineConfig[] {
  try {
    const raw = JSON.parse(localStorage.getItem(key) ?? 'null') as { version?: unknown; timelines?: unknown } | null;
    return raw?.version === STORAGE_VERSION ? sanitizeConfigs(raw.timelines) : [];
  } catch {
    return [];
  }
}

export function saveTimelines(list: readonly TimelineConfig[], key = TIMELINES_KEY): void {
  try {
    localStorage.setItem(key, JSON.stringify({ version: STORAGE_VERSION, timelines: list }));
  } catch {
    /* storage unavailable: the settings are a convenience; projects and sessions save them too */
  }
}

/**
 * The additional timeline panels ("New timeline"): their settings by panel id, opening new ones, and persistence.
 * The dock layout holds the panels themselves; this holds what each one shows. Settings are saved to local storage
 * (restored with the layout on the next start) and with projects and sessions (`contribution`).
 */
@Injectable({ providedIn: 'root' })
export class TimelineViews {
  private readonly dock = inject(DockService);
  private readonly store = inject(WorkspaceStore);
  private readonly list = signal<readonly TimelineConfig[]>(loadTimelines());

  /** Settings of every known additional timeline. */
  readonly configs = this.list.asReadonly();

  constructor() {
    effect(() => saveTimelines(this.list()));
  }

  /** The settings of a panel (the defaults for one not configured yet). Reactive. */
  config(id: string): TimelineConfig {
    return this.list().find((c) => c.id === id) ?? defaultConfig(id);
  }

  /**
   * Opens a new timeline panel (optionally preset) next to the main one and returns its id; null when the limit is
   * reached (a notice says so). Settings of panels that were closed are dropped first.
   */
  open(preset: Partial<Omit<TimelineConfig, 'id'>> = {}): string | null {
    const open = untracked(() => this.dock.openIds());
    const kept = this.list().filter((c) => open.has(c.id));
    if (kept.length >= MAX_TIMELINES) {
      this.store.setNotice(`At most ${MAX_TIMELINES} additional timelines can be open. Close one to open another.`);
      return null;
    }
    const id = nextTimelineId([...open, ...kept.map((c) => c.id)]);
    this.list.set([...kept, { ...defaultConfig(id), ...preset, id }]);
    this.dock.showBeside(id, MAIN_TIMELINE, 'below');
    return id;
  }

  /** Changes a panel's settings. */
  update(id: string, patch: Partial<Omit<TimelineConfig, 'id'>>): void {
    const current = this.config(id);
    const next = { ...current, ...patch, id };
    this.list.update((l) => (l.some((c) => c.id === id) ? l.map((c) => (c.id === id ? next : c)) : [...l, next]));
  }

  /** Renames a panel; a blank title restores the default one. */
  rename(id: string, title: string): void {
    const t = title.trim().slice(0, 80);
    this.update(id, { title: t || defaultConfig(id).title });
  }

  /** Settings saved with projects and the autosaved session: those of the open panels. */
  readonly contribution: UiStateContribution = {
    key: 'timelines',
    capture: () => {
      const open = this.dock.openIds();
      return { version: STORAGE_VERSION, timelines: this.list().filter((c) => open.has(c.id)) };
    },
    restore: (value) => {
      const v = value && typeof value === 'object' ? (value as { timelines?: unknown }) : {};
      this.list.set(sanitizeConfigs(v.timelines));
    },
    reset: () => this.list.set([]),
  };

  readonly commands: DebriefCommand[] = [{ id: 'timeline.new', title: $localize`:@@timeline.command.new:New timeline`, category: $localize`:@@timeline.command.category:View`, run: () => void this.open() }];
}
