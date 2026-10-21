import { EnvironmentProviders, Provider } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { ImtBackend } from '../../core/backend';
import {
  CollectBackend,
  EventLogBackend,
  LifecycleBackend,
  PerfmonBackend,
  PluginPackagesBackend,
  ProjectBackend,
  ToolCatalogBackend,
  ToolsBackend,
} from '../../core/tools-backend';
import { provideFakeDock, provideTinyBackend } from '../../../testing/helpers';
import { ProjectPicker } from '../../projects/project-picker';
import { RECENT_PROJECTS, RecentProjects, RecentProject } from '../../projects/recent-projects';
import { inject, signal } from '@angular/core';
import { MOCK_DEFAULTS, MockToolsBackend, MockToolsOptions } from './mock-tools-backend';
import { hasSkew } from '../../core/skew-backend';
import { MockPluginPackagesBackend, MockPluginPackagesOptions } from '../../plugins-panel/mock/mock-plugin-packages-backend';

/** The packages mock's options for tests (nothing unusual unless asked). */
export const PACKAGES_DEFAULTS: MockPluginPackagesOptions = { developerMode: false, safeMode: false, damaged: false, variant: '' };

/** In-memory recent list for tests (a signal, like `RecentService`). */
export class MemoryRecentProjects implements RecentProjects {
  private readonly state = signal<RecentProject[]>([]);
  get items(): RecentProject[] {
    return this.state();
  }
  set items(v: RecentProject[]) {
    this.state.set(v);
  }
  list = () => [...this.state()];
  add = (p: RecentProject) => this.state.update((l) => [p, ...l.filter((x) => x.path !== p.path)]);
  remove = (path: string) => this.state.update((l) => l.filter((x) => x.path !== path));
  clear = () => this.state.set([]);
}

/** A picker whose answers the test sets. */
export class FakePicker {
  open: string | null = '/p/a.imtproj';
  save: string | null = '/p/saved.imtproj';
  saveNames: string[] = [];
  pickOpen = async () => this.open;
  pickSave = async (name: string) => {
    this.saveNames.push(name);
    return this.save;
  };
}

/** Mock tools backend, tiny workspace backend, fake dock, in-memory recents and a fake picker. */
export function provideToolsTesting(
  options: Partial<MockToolsOptions> = {},
  packages: Partial<MockPluginPackagesOptions> = {},
): (Provider | EnvironmentProviders)[] {
  const tools = () => {
    const imt = inject(ImtBackend);
    return new MockToolsBackend(
      { ...MOCK_DEFAULTS, ...options },
      () => imt.workspaceSummary(),
      hasSkew(imt)
        ? (source, offsetMs) => imt.setSourceOffset({ kind: 'manual', source, offsetMs })
        : undefined,
    );
  };
  return [
    ...provideTinyBackend(),
    ...provideFakeDock(),
    { provide: ToolsBackend, useFactory: tools },
    { provide: ToolCatalogBackend, useExisting: ToolsBackend },
    { provide: PerfmonBackend, useExisting: ToolsBackend },
    { provide: CollectBackend, useExisting: ToolsBackend },
    { provide: EventLogBackend, useExisting: ToolsBackend },
    { provide: LifecycleBackend, useExisting: ToolsBackend },
    { provide: ProjectBackend, useExisting: ToolsBackend },
    { provide: PluginPackagesBackend, useFactory: () => new MockPluginPackagesBackend({ ...PACKAGES_DEFAULTS, ...packages }) },
    { provide: RECENT_PROJECTS, useValue: new MemoryRecentProjects() },
    { provide: ProjectPicker, useClass: FakePicker },
  ];
}

/** The mock backend behind the DI tokens, for assertions on `calls`. */
export function mockTools(): MockToolsBackend {
  return TestBed.inject(ToolsBackend) as unknown as MockToolsBackend;
}

/** The plugin packages mock behind the DI token, for assertions on `calls`. */
export function mockPackages(): MockPluginPackagesBackend {
  return TestBed.inject(PluginPackagesBackend) as MockPluginPackagesBackend;
}
