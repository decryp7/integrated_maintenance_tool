import { ImtPlugin } from '../../extensions/provide';

/**
 * Example UI plugin: a tree view for Windows registry exports. It is only offered while a source
 * with the "registry" importer is open, and registers one palette command.
 */
export const REGISTRY_PLUGIN: ImtPlugin = {
  views: [
    {
      id: 'registry',
      title: 'Registry',
      icon: 'folder',
      defaultPosition: 'center',
      component: () => import('./registry-view').then((m) => m.RegistryView),
      when: (summary) => summary.sources.some((s) => s.importer === 'registry'),
    },
  ],
  commands: [
    {
      id: 'registry.show',
      title: 'Show registry viewer',
      category: 'Registry',
      when: (ctx) => !!ctx.summary?.sources.some((s) => s.importer === 'registry'),
      run: (ctx) => ctx.showView('registry'),
    },
  ],
};
