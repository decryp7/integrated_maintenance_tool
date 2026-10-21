import { EnvironmentProviders, Provider } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Logger } from '../../core/logger';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { FakeContext } from '../../analysis/test-setup';
import { MemoryLogger } from '../../../testing/helpers';
import { provideToolsTesting } from '../../tools/mock/test-support';
import { MockToolsOptions } from '../../tools/mock/mock-tools-backend';
import { DumpsBackend } from '../dumps-backend';
import { DumpFileKind, DumpPicker } from '../dumps.store';
import { MOCK_DUMP_PATHS, MOCK_UMDH_PATHS, MockDumpsBackend } from '../mock/mock-dumps-backend';
import { DumpFolderPicker } from '../create-dump-view';

/** A picker whose answers the test sets. */
export class FakeDumpPicker extends DumpPicker {
  dumps: string[] = [...MOCK_DUMP_PATHS];
  umdh: string[] = [...MOCK_UMDH_PATHS];
  fail = false;
  override pick(kind: DumpFileKind): Promise<string[]> {
    if (this.fail) return Promise.reject(new Error('the dialog failed'));
    return Promise.resolve(kind === 'dump' ? this.dumps : this.umdh);
  }
}

export class FakeFolderPicker extends DumpFolderPicker {
  folder: string | null = 'C:\\Dumps';
  override pick(): Promise<string | null> {
    return Promise.resolve(this.folder);
  }
}

export interface DumpsTesting {
  backend: DumpsBackend;
  context: FakeContext;
  picker: FakeDumpPicker;
  folders: FakeFolderPicker;
}

/** Providers for the dumps specs: the mock engine, a recorded command context, fixed pickers, the tools mock. */
export function setupDumps(extra: { backend?: DumpsBackend; tools?: Partial<MockToolsOptions> } = {}): DumpsTesting {
  const doubles: DumpsTesting = {
    backend: extra.backend ?? new MockDumpsBackend(),
    context: new FakeContext(),
    picker: new FakeDumpPicker(),
    folders: new FakeFolderPicker(),
  };
  const providers: (Provider | EnvironmentProviders)[] = [
    ...provideToolsTesting({ platform: 'windows', ...extra.tools }),
    { provide: Logger, useFactory: () => new MemoryLogger() },
    { provide: DumpsBackend, useValue: doubles.backend },
    { provide: COMMAND_CONTEXT, useValue: doubles.context },
    { provide: DumpPicker, useValue: doubles.picker },
    { provide: DumpFolderPicker, useValue: doubles.folders },
  ];
  TestBed.configureTestingModule({ providers });
  return doubles;
}
