import { DebriefPlugin } from '../../extensions/provide';

/**
 * Example UI plugin: a tree view for Windows registry exports. It is only offered while a source
 * with the "registry" importer is open, and registers one palette command.
 */
export const REGISTRY_PLUGIN: DebriefPlugin = {
  views: [
    {
      id: 'registry',
      title: $localize`:@@plugins.registry.view.title:Registry`,
      plugin: $localize`:@@plugins.registry.view.plugin:Windows registry`,
      icon: 'folder',
      defaultPosition: 'center',
      layout: { area: 'document', min: { width: 420, height: 300 }, pref: { width: 700, height: 600 }, priority: 58 },
      fullHeight: true,
      component: () => import('./registry-view').then((m) => m.RegistryView),
      when: (summary) => summary.sources.some((s) => s.importer === 'registry'),
      // A viewer for the file just opened, like a document: it opens with the registry export.
      autoOpen: true,
    },
  ],
  commands: [
    {
      id: 'registry.show',
      title: $localize`:@@plugins.registry.cmd.show:Show registry viewer`,
      category: $localize`:@@plugins.registry.cmd.category:Registry`,
      when: (ctx) => !!ctx.summary?.sources.some((s) => s.importer === 'registry'),
      run: (ctx) => ctx.showView('registry'),
    },
  ],
};
