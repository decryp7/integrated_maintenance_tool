import { computed, inject, Injectable, InjectionToken, signal } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { ViewRegistry } from '../extensions/view-registry';
import { DockService } from './dock.service';
import { readLayout, SavedLayout, toEnvelope } from './layout-store';
import { BUILT_IN_PERSPECTIVES, BuiltInPerspective, PERSPECTIVE_REQUIRES } from './perspective-layouts';
import { WindowLayouts } from './window-layouts';

/** A perspective in the switcher: a built-in arrangement or a window layout the user saved. */
export interface PerspectiveInfo {
  /** `investigate`, `profiling`, `live`, `diagnostics`, or `saved:<name>`. */
  id: string;
  name: string;
  icon: string;
  builtIn: boolean;
  /** One line on what it is for (built-in ones). */
  description: string;
}

const SAVED_PREFIX = 'saved:';
export const PERSPECTIVES_KEY = 'debrief.perspectives';
const VERSION = 1;
/** Perspectives with a Ctrl+n key. */
export const PERSPECTIVE_KEYS = 9;

/** What is kept between runs: the active perspective and each perspective's arrangement as the user left it. */
export interface PerspectiveState {
  active: string;
  layouts: Readonly<Record<string, SavedLayout>>;
}

export const DEFAULT_PERSPECTIVE: BuiltInPerspective = 'investigate';
const EMPTY_STATE: PerspectiveState = { active: DEFAULT_PERSPECTIVE, layouts: {} };

/** Parses stored state; anything malformed falls back to Investigate with no remembered arrangements. */
export function parsePerspectiveState(raw: unknown): PerspectiveState {
  const v = raw as { version?: unknown; active?: unknown; layouts?: unknown } | null;
  if (!v || typeof v !== 'object' || v.version !== VERSION) return EMPTY_STATE;
  const active = typeof v.active === 'string' && v.active ? v.active : DEFAULT_PERSPECTIVE;
  const layouts: Record<string, SavedLayout> = {};
  if (v.layouts && typeof v.layouts === 'object' && !Array.isArray(v.layouts)) {
    for (const [id, envelope] of Object.entries(v.layouts as Record<string, unknown>)) {
      const saved = readLayout(envelope);
      if (saved) layouts[id] = saved;
    }
  }
  return { active, layouts };
}

export function serializePerspectiveState(state: PerspectiveState): string {
  return JSON.stringify({ version: VERSION, active: state.active, layouts: Object.fromEntries(Object.entries(state.layouts).map(([id, l]) => [id, toEnvelope(l)])) });
}

/** Where the perspective state is kept (per viewer, like the layout). Never throws. */
export interface PerspectiveStore {
  load(): PerspectiveState;
  save(state: PerspectiveState): void;
}

export class LocalStoragePerspectiveStore implements PerspectiveStore {
  constructor(private readonly key = PERSPECTIVES_KEY) {}

  load(): PerspectiveState {
    try {
      return parsePerspectiveState(JSON.parse(localStorage.getItem(this.key) ?? 'null'));
    } catch {
      return EMPTY_STATE;
    }
  }

  save(state: PerspectiveState): void {
    try {
      localStorage.setItem(this.key, serializePerspectiveState(state));
    } catch {
      /* storage full or unavailable: perspectives keep their arrangement for this run only */
    }
  }
}

export const PERSPECTIVE_STORE = new InjectionToken<PerspectiveStore>('PERSPECTIVE_STORE', {
  providedIn: 'root',
  factory: () => new LocalStoragePerspectiveStore(),
});

function builtInInfo(id: BuiltInPerspective): PerspectiveInfo {
  switch (id) {
    case 'investigate':
      return { id, builtIn: true, icon: 'search', name: $localize`:Perspective name@@dock.perspective.investigate:Investigate`, description: $localize`:@@dock.perspective.investigate.description:The timeline over the events; analysis opens below them.` };
    case 'profiling':
      return { id, builtIn: true, icon: 'profile', name: $localize`:Perspective name@@dock.perspective.profiling:Profiling`, description: $localize`:@@dock.perspective.profiling.description:Performance captures: open one or profile a process; hotspots and the flame graph follow.` };
    case 'live':
      return { id, builtIn: true, icon: 'live', name: $localize`:Perspective name@@dock.perspective.live:Live`, description: $localize`:@@dock.perspective.live.description:Follow files as they grow, with tail status and alerts below the events.` };
    case 'diagnostics':
      return { id, builtIn: true, icon: 'alert', name: $localize`:Perspective name@@dock.perspective.diagnostics:Diagnostics`, description: $localize`:@@dock.perspective.diagnostics.description:Problems, jobs, error snapshots, the store and plugin health below the events.` };
  }
}

/**
 * Perspectives (Eclipse/JetBrains): named window arrangements for a kind of work, switched from the activity bar,
 * Window ▸ Perspectives and Ctrl+1…9. Investigate, Profiling, Live and Diagnostics are built in; window layouts
 * the user saves are perspectives too. Each one remembers how the user left it (switching away and back restores
 * that); Reset perspective rebuilds it.
 */
@Injectable({ providedIn: 'root' })
export class Perspectives {
  private readonly dock = inject(DockService);
  private readonly views = inject(ViewRegistry);
  private readonly saved = inject(WindowLayouts);
  private readonly status = inject(WorkspaceStore);
  private readonly storage = inject(PERSPECTIVE_STORE);
  private state = this.storage.load();
  private readonly activeId = signal(this.state.active);
  /** Perspectives this window switched to by itself already (it does so once, see `suggest`). */
  private readonly suggested = new Set<string>();

  /** The built-in perspectives this build offers, then the saved ones. */
  readonly list = computed<PerspectiveInfo[]>(() => {
    const registered = new Set(this.views.all.map((v) => v.id));
    const builtIn = BUILT_IN_PERSPECTIVES.filter((id) => {
      const needs = PERSPECTIVE_REQUIRES[id];
      return !needs || registered.has(needs);
    }).map(builtInInfo);
    const saved = this.saved.list().map((l): PerspectiveInfo => ({ id: SAVED_PREFIX + l.name, name: l.name, icon: 'panel', builtIn: false, description: '' }));
    return [...builtIn, ...saved];
  });

  /** The active perspective (Investigate when the remembered one is gone). */
  readonly active = computed<PerspectiveInfo>(() => {
    const list = this.list();
    return list.find((p) => p.id === this.activeId()) ?? list[0] ?? builtInInfo(DEFAULT_PERSPECTIVE);
  });

  /** The active perspective's number (1-based, as in Ctrl+n), or 0 when it has none. */
  activeNumber(): number {
    return this.list().findIndex((p) => p.id === this.active().id) + 1;
  }

  /** Perspective number `n` (1-based, Ctrl+n). */
  at(n: number): PerspectiveInfo | undefined {
    return this.list()[n - 1];
  }

  /**
   * Switches to a perspective: the current arrangement is remembered for the one being left, and the target's
   * remembered arrangement (or its built-in/saved one) is shown. Returns false for an unknown id.
   */
  switchTo(id: string, options: { quiet?: boolean } = {}): boolean {
    const target = this.list().find((p) => p.id === id);
    if (!target) return false;
    const current = this.active().id;
    if (current === id) return true;
    const layout = this.dock.currentLayout();
    if (layout) this.remember(current, layout);
    const remembered = this.state.layouts[id];
    if (!remembered || !this.dock.applyLayout(remembered)) this.show(target);
    this.activeId.set(id);
    this.persist({ ...this.state, active: id });
    if (!options.quiet) this.status.setNotice($localize`:@@dock.perspective.switched:Perspective: ${target.name}:name:`);
    return true;
  }

  /** Switches to perspective `n` (Ctrl+n). */
  switchToAt(n: number): boolean {
    const p = this.at(n);
    return p ? this.switchTo(p.id) : false;
  }

  /** Rebuilds the active perspective as it comes (Window ▸ Perspectives ▸ Reset perspective). */
  reset(): void {
    const active = this.active();
    const layouts = { ...this.state.layouts };
    delete layouts[active.id];
    this.persist({ ...this.state, layouts });
    this.show(active);
    this.status.setNotice($localize`:@@dock.perspective.reset:Perspective ${active.name}:name: reset`);
  }

  /**
   * Switches to a perspective because of what the user just did (a performance capture opened: Profiling), once
   * per window and only from another perspective; the notice offers Undo, which goes back.
   */
  suggest(id: string): void {
    if (this.suggested.has(id) || this.active().id === id || !this.list().some((p) => p.id === id)) return;
    this.suggested.add(id);
    const from = this.active();
    if (!this.switchTo(id, { quiet: true })) return;
    const to = this.active();
    this.status.setNotice($localize`:@@dock.perspective.suggested:Switched to the ${to.name}:name: perspective`, {
      label: $localize`:@@dock.perspective.undo:Undo`,
      run: () => void this.switchTo(from.id, { quiet: true }),
    });
  }

  /** Saves the current arrangement as a perspective (a named window layout) and makes it the active one. */
  saveCurrent(name: string): string | null {
    const saved = this.saved.saveCurrent(name);
    if (!saved) return null;
    const id = SAVED_PREFIX + saved;
    const layouts = { ...this.state.layouts };
    delete layouts[id];
    this.activeId.set(id);
    this.persist({ active: id, layouts });
    return saved;
  }

  /** Renames a saved perspective, keeping its remembered arrangement and the active choice; false if refused. */
  rename(from: string, to: string): boolean {
    const before = this.saved.list().map((l) => l.name);
    if (!this.saved.rename(from, to)) return false;
    const after = this.saved.list().find((l) => !before.includes(l.name))?.name;
    if (!after) return true;
    const layouts = { ...this.state.layouts };
    const old = SAVED_PREFIX + from;
    const moved = layouts[old];
    delete layouts[old];
    if (moved) layouts[SAVED_PREFIX + after] = moved;
    const active = this.activeId() === old ? SAVED_PREFIX + after : this.state.active;
    this.activeId.set(active);
    this.persist({ active, layouts });
    return true;
  }

  /** Deletes a saved perspective; when it was active the window keeps its arrangement and Investigate is named active. */
  remove(name: string): void {
    const id = SAVED_PREFIX + name;
    this.saved.remove(name);
    const layouts = { ...this.state.layouts };
    delete layouts[id];
    const active = this.activeId() === id ? DEFAULT_PERSPECTIVE : this.state.active;
    this.activeId.set(active);
    this.persist({ active, layouts });
  }

  /** Moves a saved perspective up (-1) or down (+1) in the list (and so to another Ctrl+n). */
  move(name: string, delta: -1 | 1): void {
    this.saved.move(name, delta);
  }

  private show(p: PerspectiveInfo): void {
    if (p.builtIn) {
      this.dock.buildPerspective(p.id as BuiltInPerspective);
      return;
    }
    if (!this.saved.apply(p.name)) this.dock.buildPerspective(DEFAULT_PERSPECTIVE);
  }

  private remember(id: string, layout: SavedLayout): void {
    this.state = { ...this.state, layouts: { ...this.state.layouts, [id]: layout } };
  }

  private persist(state: PerspectiveState): void {
    this.state = state;
    this.storage.save(state);
  }
}
