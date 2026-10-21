import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Logger } from '../../core/logger';
import { MockAiBackend, MockAiOptions } from '../../core/mock/mock-ai-backend';
import { MockJobs, resetMockJobs } from '../../jobs/mock-jobs';
import { MemoryLogger } from '../../../testing/helpers';
import { AiBackend, AiError } from '../ai-backend';
import { AiStatusStore } from '../ai-status.store';
import { AiImageStore } from './ai-image.store';

const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13]);

async function setup(options: MockAiOptions = {}) {
  const backend = new MockAiBackend({ stepMs: 0, ...options });
  TestBed.configureTestingModule({ providers: [{ provide: AiBackend, useValue: backend }, { provide: Logger, useClass: MemoryLogger }] });
  const status = TestBed.inject(AiStatusStore);
  await status.refresh();
  return { backend, status, store: TestBed.inject(AiImageStore) };
}

describe('AiImageStore', () => {
  beforeEach(() => resetMockJobs(new MockJobs({ visibleMs: 0, attentionMs: 0, failOnce: new Set() })));
  afterEach(() => {
    TestBed.resetTestingModule();
    resetMockJobs();
  });

  it('opens a picked file and pasted bytes, switches and closes them', async () => {
    const { store, backend } = await setup();
    const a = await store.openPath('C:\\shots\\dialog.png');
    expect(a?.name).toBe('dialog.png');
    expect(store.current()?.id).toBe(a?.id);
    const paste = vi.spyOn(backend, 'imagePaste');
    const b = await store.pasteBytes(PNG, 'clip.png');
    expect(paste).toHaveBeenCalledWith('iVBORw0KGgoAAAAN', 'clip.png');
    expect(store.images().map((i) => i.name)).toEqual(['dialog.png', 'clip.png']);
    expect(store.current()?.id).toBe(b?.id);
    // Opened for the chat: the image view keeps showing what it showed.
    const c = await store.pasteFile(new File([PNG], 'chat.png', { type: 'image/png' }), false);
    expect(c?.name).toBe('chat.png');
    expect(store.current()?.id).toBe(b?.id);
    store.select(a!.id);
    expect(store.current()?.id).toBe(a!.id);
    store.select(999);
    expect(store.current()?.id).toBe(a!.id);
    const close = vi.spyOn(backend, 'imageClose');
    await store.close(a!.id);
    expect(close).toHaveBeenCalledWith(a!.id);
    expect(store.current()?.id).toBe(c!.id);
    await store.close(b!.id);
    expect(store.current()?.id).toBe(c!.id);
  });

  it('reports a failed open and retries it', async () => {
    const { store, backend } = await setup();
    expect(await store.openPath('/x/notes.txt')).toBeNull();
    expect(store.openError()).toContain('PNG, JPEG');
    const open = vi.spyOn(backend, 'imageOpen').mockResolvedValueOnce({ ...(await backend.imageOpen('/x/a.png')), name: 'notes.txt' });
    expect((await store.retryOpen())?.name).toBe('notes.txt');
    expect(open).toHaveBeenCalledWith('/x/notes.txt');
    expect(store.openError()).toBeNull();
    expect(await store.pasteBytes(new Uint8Array([1, 2, 3]), null)).toBeNull();
    expect(store.openError()).toContain('not a PNG');
    store.dismissOpenError();
    expect(store.openError()).toBeNull();
  });

  it('refuses images over 32 MB before reading them', async () => {
    const { store, backend } = await setup();
    const paste = vi.spyOn(backend, 'imagePaste');
    const huge = { size: 33 * 1024 * 1024, name: 'huge.png', arrayBuffer: vi.fn() } as unknown as File;
    expect(await store.pasteFile(huge)).toBeNull();
    expect(store.openError()).toContain('32 MB');
    expect(huge.arrayBuffer).not.toHaveBeenCalled();
    expect(await store.retryOpen()).toBeNull();
    expect(await store.pasteBytes(new Uint8Array(33 * 1024 * 1024), 'big.png')).toBeNull();
    expect(paste).not.toHaveBeenCalled();
    resetMockJobs();
    expect(await TestBed.inject(AiImageStore).retryOpen()).toBeNull();
  });

  it('explains an image and finds its related logs as AI jobs', async () => {
    const { store } = await setup();
    const img = (await store.openPath('/shots/dialog.png'))!;
    const explaining = store.explain(img.id, 'what is this?');
    expect(store.currentExplain()?.running).toBe(true);
    await explaining;
    expect(store.currentExplain()?.running).toBe(false);
    expect(store.currentExplain()?.value?.text).toContain('You asked: "what is this?"');
    expect(store.currentExplain()?.jobId).toBeGreaterThan(0);
    await store.findRelated(img.id);
    const r = store.currentRelated()!;
    expect(r.error).toBeNull();
    expect(r.value?.result.links.length).toBeGreaterThan(0);
    expect(store.links()[0]).toMatchObject({ imageId: img.id, imageName: 'dialog.png' });
    expect(store.jobImages()[r.jobId!]).toEqual({ imageId: img.id, imageName: 'dialog.png' });
    // Unknown images are ignored.
    await store.findRelated(999);
    expect(store.relatedResults()[999]).toBeUndefined();
  });

  it('shows a Jobs result with its image, or on its own once the image is closed', async () => {
    const { store } = await setup();
    const img = (await store.openPath('/shots/dialog.png'))!;
    await store.findRelated(img.id);
    const jobId = store.currentRelated()!.jobId!;
    const outcome = store.currentRelated()!.value!;
    const other = (await store.openPath('/shots/other.png'))!;
    expect(store.showOutcome(jobId, outcome)).toBe(true);
    expect(store.current()?.id).toBe(img.id);
    store.select(other.id);
    await store.close(img.id);
    expect(store.showOutcome(jobId, outcome)).toBe(true);
    expect(store.current()).toBeNull();
    expect(store.detached()).toMatchObject({ jobId, imageName: 'dialog.png' });
    expect(store.links()[0]).toMatchObject({ imageId: null, imageName: 'dialog.png' });
    expect(store.showOutcome(77, outcome)).toBe(true);
    expect(store.detached()?.imageName).toBeNull();
    expect(store.showOutcome(78, { nope: true })).toBe(false);
    store.select(other.id);
    expect(store.detached()).toBeNull();
  });

  it('reports failed and unreadable answers with the reason, and keeps the last answer when cancelled', async () => {
    const { store, backend } = await setup();
    const img = (await store.openPath('/shots/dialog.png'))!;
    vi.spyOn(backend, 'start').mockRejectedValueOnce(new AiError('busy', '20 ai jobs are already waiting'));
    await store.explain(img.id);
    expect(store.currentExplain()?.error).toBe('20 ai jobs are already waiting');
    await store.explain(img.id);
    const answer = store.currentExplain()!.value;
    expect(answer).not.toBeNull();
    const { JobFollow } = await import('../../jobs/job-follow');
    const follow = TestBed.inject(JobFollow);
    vi.spyOn(follow, 'run').mockResolvedValueOnce(null);
    await store.explain(img.id);
    expect(store.currentExplain()).toMatchObject({ running: false, error: null, value: answer });
    vi.spyOn(follow, 'run').mockResolvedValueOnce({ type: 'narrative', narrative: { bad: 1 } });
    await store.explain(img.id);
    expect(store.currentExplain()?.error).toContain('could not be read');
    vi.spyOn(follow, 'run').mockResolvedValueOnce({ type: 'chat', sessionId: 'x' });
    await store.findRelated(img.id);
    expect(store.currentRelated()?.error).toContain('could not be read');
    // The image was closed in the engine: the job fails with its reason.
    await backend.imageClose(img.id);
    await store.findRelated(img.id);
    expect(store.currentRelated()?.error).toContain('no longer open');
  });

  it('does not ask a text-only model about images and offers its image support', async () => {
    const { store, backend, status } = await setup({ vision: 'missing' });
    expect(status.status()?.vision).toBe(false);
    expect(store.visionMissing()).toBe(true);
    const img = (await store.openPath('/shots/dialog.png'))!;
    const start = vi.spyOn(backend, 'start');
    await store.findRelated(img.id);
    expect(start).not.toHaveBeenCalled();
    expect(store.noVision()).toBe(true);
    expect((await store.loadActiveModel())?.projectorDownloadBytes).toBe(640 * 1024 * 1024);
    const install = vi.spyOn(backend, 'installProjector');
    await store.installProjector();
    expect(install).toHaveBeenCalledWith('Qwen3.5-4B-Q4_K_M.gguf');
    expect(store.projector()).toMatchObject({ running: false, error: null });
    expect(store.noVision()).toBe(false);
    expect(status.status()?.vision).toBe(true);
    expect(store.activeModel()?.vision).toBe(true);
    await store.findRelated(img.id);
    expect(start).toHaveBeenCalledWith({ op: 'relatedLogs', imageId: img.id });
  });

  it('turns a NoVision refusal into the no-vision state, and reports a failed download', async () => {
    const { store, backend } = await setup({ vision: 'missing', net: 'tls' });
    const img = (await store.openPath('/shots/dialog.png'))!;
    vi.spyOn(TestBed.inject(AiStatusStore), 'status').mockReturnValue(null);
    vi.spyOn(backend, 'start').mockRejectedValueOnce(new AiError('noVision', 'text only'));
    await store.explain(img.id);
    expect(store.noVision()).toBe(true);
    store.dismissNoVision();
    expect(store.noVision()).toBe(false);
    store.refuseNoVision();
    expect(store.noVision()).toBe(true);
    await store.installProjector();
    expect(store.projector().error).toBeTruthy();
    vi.spyOn(backend, 'installProjector').mockRejectedValueOnce(new AiError('notFound', 'no projector'));
    await store.installProjector();
    expect(store.projector().error).toBe('no projector');
  });

  it('cannot add image support without an active model', async () => {
    const { store, backend } = await setup({ mode: 'setup' });
    const install = vi.spyOn(backend, 'installProjector');
    await store.installProjector();
    expect(install).not.toHaveBeenCalled();
    vi.spyOn(backend, 'models').mockRejectedValueOnce(new Error('down'));
    expect(await store.loadActiveModel()).toBeNull();
  });
});
