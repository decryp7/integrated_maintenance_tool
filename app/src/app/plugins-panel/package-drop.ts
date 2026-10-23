import { inject, Injector } from '@angular/core';
import { DroppedFilesHandler } from '../core/dropped-files';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { PackageReviewOpener } from './package-review-dialog';
import { isPackagePath, PluginPackagesStore } from './plugin-packages.store';

/** The Plugins panel's view id (see builtin-views.ts). */
const PLUGINS_VIEW = 'plugins';

/**
 * The drag-and-drop claim of plugin packages: `.debriefplugin` files are never opened as logs. The first one is reviewed
 * (the review dialog opens over the Plugins panel); the shell granted the dropped paths already.
 */
export function pluginPackageDropHandler(): DroppedFilesHandler {
  const store = inject(PluginPackagesStore);
  const injector = inject(Injector);
  return async (paths) => {
    const claimed = paths.filter(isPackagePath);
    const [first] = claimed;
    if (first === undefined) return [];
    injector.get(PackageReviewOpener);
    injector.get(COMMAND_CONTEXT).showView(PLUGINS_VIEW);
    await store.reviewPackage(first);
    return claimed;
  };
}
