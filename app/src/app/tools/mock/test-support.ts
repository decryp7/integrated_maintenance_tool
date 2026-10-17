import { EnvironmentProviders, Provider } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { ImtBackend } from '../../core/backend';
import {
  CollectBackend,
  LifecycleBackend,
  PerfmonBackend,
  ProjectBackend,
  ToolCatalogBackend,
  ToolsBackend,
} from '../../core/tools-backend';
import { provideFakeDock, provideTinyBackend } from '../../../testing/helpers';
import { ProjectPicker } from '../../projects/project-picker';
import { RECENT_PROJECTS, RecentProjects, RecentProject } from '../../projects/recent-projects';
import { inject } from '@angular/core';
import { MOCK_DEFAULTS, MockToolsBackend, MockToolsOptions } from './mock-tools-backend';
import { hasSkew } from '../../core/skew-backend';

/** In-memory recent list for tests. */
export class MemoryRecentProjects implements RecentProjects {
  items: RecentProject[] = [];
  list = () => [...this.items];
  add = (p: RecentProject) =>
    void (this.items = [p, ...this.items.filter((x) => x.path !== p.path)]);
  remove = (path: string) => void (this.items = this.items.filter((x) => x.path !== path));
  clear = () => void (this.items = []);
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
    { provide: LifecycleBackend, useExisting: ToolsBackend },
    { provide: ProjectBackend, useExisting: ToolsBackend },
    { provide: RECENT_PROJECTS, useValue: new MemoryRecentProjects() },
    { provide: ProjectPicker, useClass: FakePicker },
  ];
}

/** The mock backend behind the DI tokens, for assertions on `calls`. */
export function mockTools(): MockToolsBackend {
  return TestBed.inject(ToolsBackend) as unknown as MockToolsBackend;
}
