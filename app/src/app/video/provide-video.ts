import { EnvironmentProviders, inject, Injectable, Injector, makeEnvironmentProviders, provideAppInitializer } from '@angular/core';
import { VideoView as VideoViewDto } from '../core/generated/video';
import { DROPPED_FILES_HANDLERS, DroppedFilesHandler } from '../core/dropped-files';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { CommandContext, DEBRIEF_COMMANDS, DEBRIEF_VIEWS, DebriefCommand, DebriefView } from '../extensions/tokens';
import { provideVideoBackend } from './video-backend';
import { VideoStore } from './video.store';

/** The Video view's id. */
export const VIDEO_VIEW_ID = 'video';

/** The Video view (same extension point as plugin views). */
export function videoViews(): DebriefView[] {
  return [
    {
      id: VIDEO_VIEW_ID,
      category: 'Diagnostics',
      title: $localize`:@@video.views.video:Video`,
      icon: 'image',
      defaultPosition: 'center', layout: { area: 'document', min: { width: 420, height: 240 }, pref: { width: 640, height: 400 }, priority: 55 },
      component: () => import('./video-view').then((m) => m.VideoView),
    },
  ];
}

/** Shows the Video view once something opened, or when an open failed (the view says why, with Retry). */
export function showOpened(ctx: CommandContext, opened: readonly VideoViewDto[], failed: boolean): void {
  if (opened.length || failed) ctx.showView(VIDEO_VIEW_ID);
}

/** File ▸ Open video… and Video: show. */
@Injectable({ providedIn: 'root' })
export class VideoCommands {
  private readonly store = inject(VideoStore);

  readonly commands: DebriefCommand[] = [
    { id: 'video.open', title: $localize`:@@video.cmd.open:Open video…`, category: 'File', run: (ctx) => this.open(ctx) },
    { id: 'video.show', title: $localize`:@@video.cmd.show:Video: show`, category: $localize`:@@video.category:Video`, run: (ctx) => ctx.showView(VIDEO_VIEW_ID) },
  ];

  async open(ctx: CommandContext): Promise<void> {
    const opened = await this.store.pickAndOpen();
    showOpened(ctx, opened, !!this.store.errors().open);
  }
}

/** Loads the videos the project already has. */
export function startVideo(): void {
  void inject(VideoStore).refresh();
}

/** The drag-and-drop claim of videos: files the engine sniffs as videos are analysed here; the rest go on. */
export function videoDropHandler(): DroppedFilesHandler {
  const store = inject(VideoStore);
  const injector = inject(Injector);
  return async (paths) => {
    const { claimed, opened } = await store.openDropped(paths);
    if (claimed.length) showOpened(injector.get(COMMAND_CONTEXT), opened, !!store.errors().open);
    return claimed;
  };
}

/** One line in `app.config.ts`: backend, view, commands and drag-and-drop of videos. */
export function provideVideo(): EnvironmentProviders {
  return makeEnvironmentProviders([
    ...provideVideoBackend(),
    { provide: DEBRIEF_VIEWS, multi: true, useFactory: videoViews },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(VideoCommands).commands },
    { provide: DROPPED_FILES_HANDLERS, multi: true, useFactory: videoDropHandler },
    provideAppInitializer(() => startVideo()),
  ]);
}
