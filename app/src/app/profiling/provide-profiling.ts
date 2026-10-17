import { EnvironmentProviders, inject, Injectable, Injector, makeEnvironmentProviders, provideAppInitializer } from '@angular/core';
import { CaptureSummary } from '../core/api.types';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { DROPPED_FILES_HANDLERS, DroppedFilesHandler } from '../core/dropped-files';
import { CommandContext, IMT_COMMANDS, IMT_VIEWS, ImtCommand, ImtView } from '../extensions/tokens';
import { UI_STATE_CONTRIBUTIONS } from '../lifecycle/ui-state';
import { CapturesStore } from './captures.store';
import { ProfilingPersistence } from './profiling-persistence';
import { provideProfilingBackend } from './profiling-backend';
import { PROFILING_VIEW_IDS as V } from './profiling-navigator';
import { isHeap, isProfile } from './profiling-format';

/** The profiling views. Same extension point as plugin views; none opens until a capture is. */
export const PROFILING_VIEWS: ImtView[] = [
  { id: V.captures, category: 'Profiling', title: 'Profiles', icon: 'files', defaultPosition: 'right', component: () => import('./captures/captures-view').then((m) => m.CapturesView) },
  { id: V.hotspots, category: 'Profiling', title: 'Hotspots', icon: 'columns', defaultPosition: 'center', component: () => import('./hotspots/hotspots-view').then((m) => m.HotspotsView) },
  { id: V.tree, category: 'Profiling', title: 'Call tree', icon: 'group', defaultPosition: 'center', component: () => import('./tree/call-tree-view').then((m) => m.CallTreeView) },
  { id: V.inverted, category: 'Profiling', menu: 'palette', title: 'Inverted tree', icon: 'group', defaultPosition: 'center', component: () => import('./tree/call-tree-view').then((m) => m.InvertedTreeView) },
  { id: V.butterfly, category: 'Profiling', menu: 'palette', title: 'Callers and callees', icon: 'selector', defaultPosition: 'right', component: () => import('./butterfly/butterfly-view').then((m) => m.ButterflyView) },
  { id: V.flame, category: 'Profiling', title: 'Flame graph', icon: 'chart', defaultPosition: 'center', component: () => import('./flame/flame-view').then((m) => m.FlameView) },
  { id: V.compare, category: 'Profiling', title: 'Compare profiles', icon: 'columns', defaultPosition: 'center', component: () => import('./compare/compare-view').then((m) => m.CompareView) },
  { id: V.threads, category: 'Profiling', title: 'Thread timeline', icon: 'timeline', defaultPosition: 'bottom', component: () => import('./threads/thread-timeline-view').then((m) => m.ThreadTimelineView) },
  { id: V.findings, category: 'Profiling', title: 'Profile findings', icon: 'alert', defaultPosition: 'right', component: () => import('./findings/findings-view').then((m) => m.FindingsView) },
  { id: V.heap, category: 'Profiling', title: 'Memory', icon: 'metrics', defaultPosition: 'center', component: () => import('./heap/heap-view').then((m) => m.HeapView) },
];

const CATEGORY = 'Profiling';

/** Palette commands (and the File menu's "Open profile…"). */
@Injectable({ providedIn: 'root' })
export class ProfilingCommands {
  private readonly captures = inject(CapturesStore);

  private show(id: string, title: string, view: string): ImtCommand {
    return { id, title, category: CATEGORY, run: (ctx) => ctx.showView(view) };
  }

  readonly commands: ImtCommand[] = [
    { id: 'profiling.open', title: 'Open profile…', category: 'File', run: (ctx) => this.open(ctx) },
    this.show('profiling.captures', 'Profiling: show profiles', V.captures),
    this.show('profiling.hotspots', 'Profiling: hotspots', V.hotspots),
    this.show('profiling.tree', 'Profiling: call tree', V.tree),
    this.show('profiling.inverted', 'Profiling: inverted tree (bottom-up)', V.inverted),
    this.show('profiling.butterfly', 'Profiling: callers and callees', V.butterfly),
    this.show('profiling.flame', 'Profiling: flame graph', V.flame),
    this.show('profiling.compare', 'Profiling: compare with a baseline (differential flame graph)', V.compare),
    this.show('profiling.threads', 'Profiling: thread timeline', V.threads),
    this.show('profiling.findings', 'Profiling: findings', V.findings),
    this.show('profiling.heap', 'Profiling: memory (heap snapshots)', V.heap),
  ];

  /** Opens captures and shows the most useful view for what was opened. */
  async open(ctx: CommandContext): Promise<void> {
    showOpened(ctx, await this.captures.pickAndOpen());
    if (this.captures.error()) ctx.showView(V.captures);
  }
}

/** Profiles open on the hotspots, heap snapshots on the memory view; the captures list follows. */
export function showOpened(ctx: CommandContext, opened: CaptureSummary[]): void {
  if (!opened.length) return;
  ctx.showView(V.captures);
  if (opened.some(isProfile)) ctx.showView(V.hotspots);
  else if (opened.some(isHeap)) ctx.showView(V.heap);
}

/** Loads the captures the engine already has open in this window. */
export function startProfiling(): void {
  void inject(CapturesStore).refresh();
}

/**
 * The drag-and-drop claim of profiling: files the engine sniffs as profiles or heap snapshots open
 * here (and show their view, or their error in the Profiles view); the rest go to the log import.
 */
export function profilingDropHandler(): DroppedFilesHandler {
  const captures = inject(CapturesStore);
  const injector = inject(Injector);
  return async (paths) => {
    const { claimed, opened } = await captures.openDropped(paths);
    const ctx = injector.get(COMMAND_CONTEXT);
    showOpened(ctx, opened);
    if (captures.error()) ctx.showView(V.captures);
    return claimed;
  };
}

/** One line in `app.config.ts`: profiling backend, views, commands, drag-and-drop of captures and their persistence. */
export function provideProfiling(): EnvironmentProviders {
  return makeEnvironmentProviders([
    ...provideProfilingBackend(),
    { provide: IMT_VIEWS, multi: true, useValue: PROFILING_VIEWS },
    { provide: IMT_COMMANDS, multi: true, useFactory: () => inject(ProfilingCommands).commands },
    { provide: DROPPED_FILES_HANDLERS, multi: true, useFactory: profilingDropHandler },
    { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => [inject(ProfilingPersistence).contribution] },
    provideAppInitializer(() => startProfiling()),
  ]);
}
