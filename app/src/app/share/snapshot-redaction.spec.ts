import { TestBed } from '@angular/core/testing';
import { Logger } from '../core/logger';
import { WorkspaceStore } from '../core/workspace.store';
import { ErrorSnapshotService, SNAPSHOT_CLOCK, SNAPSHOT_TEXT_FILTER } from '../diagnostics/error-snapshot.service';
import { MockSnapshotBackend } from '../diagnostics/mock/mock-snapshot-backend';
import { ScreenCapture } from '../diagnostics/screen-capture';
import { SnapshotBackend } from '../diagnostics/snapshot-backend';
import { MemoryLogger } from '../../testing/helpers';
import { ShareBackend } from './share-backend';
import { snapshotRedaction, UNREDACTED_PLACEHOLDER } from './snapshot-redaction';
import { at } from '../../testing/must';

class Screen extends ScreenCapture {
  async capture() {
    return { base64: 'iVBORw0KGgo=', mime: 'image/png' as const, bytes: 8 };
  }
}

function setup(redactedView: boolean, preview: () => Promise<{ after: string }>) {
  const backend = new MockSnapshotBackend();
  TestBed.configureTestingModule({
    providers: [
      { provide: SnapshotBackend, useValue: backend },
      { provide: ScreenCapture, useValue: new Screen() },
      { provide: Logger, useClass: MemoryLogger },
      { provide: SNAPSHOT_CLOCK, useValue: { now: () => 0, nextFrame: async () => undefined } },
      { provide: WorkspaceStore, useValue: { summary: () => ({ redactedView }) } },
      { provide: ShareBackend, useValue: { previewRedaction: preview } },
      { provide: SNAPSHOT_TEXT_FILTER, useFactory: snapshotRedaction },
    ],
  });
  return { backend, service: TestBed.inject(ErrorSnapshotService) };
}

describe('error snapshots and the redacted view', () => {
  it('redacts the stored message and stack through the engine while the view is redacted', async () => {
    const preview = vi.fn(async () => ({ after: 'masked' }));
    const { backend, service } = setup(true, preview);
    await service.capture('unhandled-error', { error: new Error('mail a@b.io') });
    const saves = await backend.list();
    const saved = at(saves, 0);
    expect(saved.message).toBe('masked');
    expect(saved.stack).toBe('masked');
    expect(preview).toHaveBeenCalledTimes(2);
  });

  it('keeps texts as they are while the view is not redacted, and never stores text it could not redact', async () => {
    const preview = vi.fn(async () => ({ after: 'masked' }));
    const plain = setup(false, preview);
    await plain.service.capture('manual');
    expect((await plain.backend.list())[0]?.message).toBe('manual');
    expect(preview).not.toHaveBeenCalled();
    TestBed.resetTestingModule();
    const failing = setup(true, async () => {
      throw new Error('no engine');
    });
    await failing.service.capture('manual');
    const saves = await failing.backend.list();
    const saved = at(saves, 0);
    expect(saved.message).toBe(UNREDACTED_PLACEHOLDER);
    expect(saved.stack ?? null).toBeNull();
  });
});
