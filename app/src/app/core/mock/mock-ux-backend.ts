import { ImportProgress, ProfileDoc, ProfileVersionInfo } from '../api.types';
import type { ImtBackend } from '../backend';
import type { UxBackend } from '../ux-backend';
import { MockBackend } from './mock-backend';
import { IDLE_PROGRESS, MOCK_DEMO_PATHS } from './mock-import';

/**
 * Browser stand-in for the ease-of-use commands, over the mock workspace (progress, history and the demo
 * live in `MockBackend`). Plausible, never the reference: the engine is.
 */
export class MockUxBackend implements UxBackend {
  constructor(private readonly imt: ImtBackend) {}

  private mock(): MockBackend | null {
    return this.imt instanceof MockBackend ? this.imt : null;
  }

  importProgress(): Promise<ImportProgress> {
    return this.mock()?.importProgress() ?? Promise.resolve({ ...IDLE_PROGRESS });
  }

  cancelImport(): Promise<void> {
    return this.mock()?.cancelImport() ?? Promise.resolve();
  }

  profileHistory(): Promise<ProfileVersionInfo[]> {
    return this.mock()?.profileHistory() ?? Promise.resolve([]);
  }

  profileVersion(file: string): Promise<ProfileDoc> {
    return this.mock()?.profileVersion(file) ?? Promise.reject(new Error('no profile history'));
  }

  demoPaths(): Promise<string[]> {
    return Promise.resolve([...MOCK_DEMO_PATHS]);
  }
}
