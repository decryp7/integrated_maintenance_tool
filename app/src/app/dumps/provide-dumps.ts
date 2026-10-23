import { EnvironmentProviders, inject, Injectable, Injector, makeEnvironmentProviders, provideAppInitializer } from '@angular/core';
import { DumpDocSummary } from '../core/api.types';
import { DROPPED_FILES_HANDLERS, DroppedFilesHandler } from '../core/dropped-files';
import { TOOL_IDS } from '../core/tools.types';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { CommandContext, DEBRIEF_COMMANDS, DEBRIEF_VIEWS, DebriefCommand, DebriefView } from '../extensions/tokens';
import { UI_STATE_CONTRIBUTIONS } from '../lifecycle/ui-state';
import { ToolCatalog } from '../tools/tool-catalog.store';
import { provideDumpsBackend } from './dumps-backend';
import { DumpsPersistence } from './dumps-persistence';
import { DumpFileKind, DumpsStore } from './dumps.store';

/** The views' ids. */
export const DUMP_VIEW_IDS = { dump: 'dump', umdh: 'umdh', capture: 'dump-capture' } as const;
const V = DUMP_VIEW_IDS;

/** The dump views (same extension point as plugin views). */
export function dumpsViews(tools: { listed(id: string): boolean }): DebriefView[] {
  return [
    { id: V.dump, category: 'Diagnostics', title: $localize`:@@dumps.views.dump:Dump`, icon: 'alert', defaultPosition: 'center', layout: { area: 'document', min: { width: 420, height: 240 }, pref: { width: 640, height: 400 }, priority: 55 }, component: () => import('./dump-view').then((m) => m.DumpView) },
    { id: V.umdh, category: 'Diagnostics', title: $localize`:@@dumps.views.umdh:UMDH heap growth`, icon: 'metrics', defaultPosition: 'center', layout: { area: 'document', min: { width: 420, height: 240 }, pref: { width: 640, height: 400 }, priority: 55 }, component: () => import('./umdh-view').then((m) => m.UmdhView) },
    { id: V.capture, category: 'Tools', title: $localize`:@@dumps.views.capture:Create dump`, icon: 'file', defaultPosition: 'center', layout: { area: 'document', min: { width: 420, height: 240 }, pref: { width: 640, height: 400 }, priority: 55 }, component: () => import('./create-dump-view').then((m) => m.CreateDumpView), enabled: () => tools.listed(TOOL_IDS.dumpCapture) },
  ];
}

/** Shows the view for what was opened: the Dump view for dumps, the UMDH view for heap logs. */
export function showOpened(ctx: CommandContext, opened: readonly DumpDocSummary[]): void {
  if (opened.some((d) => d.kind !== 'dump')) ctx.showView(V.umdh);
  if (opened.some((d) => d.kind === 'dump')) ctx.showView(V.dump);
}

/** File ▸ Open dump… / Open UMDH snapshots…, Tools ▸ Create dump of process…, and the views. */
@Injectable({ providedIn: 'root' })
export class DumpsCommands {
  private readonly store = inject(DumpsStore);
  private readonly catalog = inject(ToolCatalog);

  readonly commands: DebriefCommand[] = [
    { id: 'dumps.open', title: $localize`:@@dumps.cmd.open:Open dump…`, category: 'File', run: (ctx) => this.open(ctx, 'dump') },
    { id: 'dumps.openUmdh', title: $localize`:@@dumps.cmd.openUmdh:Open UMDH snapshots…`, category: 'File', run: (ctx) => this.open(ctx, 'umdh') },
    { id: 'dumps.capture', title: $localize`:@@dumps.cmd.capture:Create dump of process…`, category: $localize`:@@tools.category.tools:Tools`, when: () => this.catalog.listed(TOOL_IDS.dumpCapture), run: (ctx) => ctx.showView(V.capture) },
    { id: 'dumps.show', title: $localize`:@@dumps.cmd.show:Dumps: show the dump`, category: $localize`:@@dumps.category:Dumps`, run: (ctx) => ctx.showView(V.dump) },
    { id: 'dumps.showUmdh', title: $localize`:@@dumps.cmd.showUmdh:Dumps: UMDH heap growth`, category: $localize`:@@dumps.category:Dumps`, run: (ctx) => ctx.showView(V.umdh) },
  ];

  async open(ctx: CommandContext, kind: DumpFileKind): Promise<void> {
    const opened = await this.store.pickAndOpen(kind);
    showOpened(ctx, opened);
    if (!opened.length && this.store.error()) ctx.showView(kind === 'dump' ? V.dump : V.umdh);
  }
}

/** Loads the documents the engine already has open in this window. */
export function startDumps(): void {
  void inject(DumpsStore).refresh();
}

/** The drag-and-drop claim of dumps: files the engine sniffs as dumps or UMDH logs open here; the rest go on. */
export function dumpsDropHandler(): DroppedFilesHandler {
  const store = inject(DumpsStore);
  const injector = inject(Injector);
  return async (paths) => {
    const { claimed, opened } = await store.openDropped(paths);
    const ctx = injector.get(COMMAND_CONTEXT);
    showOpened(ctx, opened);
    if (claimed.length && !opened.length && store.error()) ctx.showView(V.dump);
    return claimed;
  };
}

/** One line in `app.config.ts`: backend, views, commands, drag-and-drop of dumps and their persistence. */
export function provideDumps(): EnvironmentProviders {
  return makeEnvironmentProviders([
    ...provideDumpsBackend(),
    { provide: DEBRIEF_VIEWS, multi: true, useFactory: () => dumpsViews(inject(ToolCatalog)) },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(DumpsCommands).commands },
    { provide: DROPPED_FILES_HANDLERS, multi: true, useFactory: dumpsDropHandler },
    { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => [inject(DumpsPersistence).contribution] },
    provideAppInitializer(() => startDumps()),
  ]);
}
