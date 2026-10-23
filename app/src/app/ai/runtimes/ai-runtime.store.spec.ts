import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MockAiBackend, MockAiOptions } from '../../core/mock/mock-ai-backend';
import { settle } from '../../../testing/helpers';
import { AiBackend, AiError } from '../ai-backend';
import { AiRuntimeStore } from './ai-runtime.store';

function setup(options: MockAiOptions = {}) {
  const backend = new MockAiBackend({ stepMs: 0, ...options });
  TestBed.configureTestingModule({ providers: [{ provide: AiBackend, useValue: backend }] });
  return { backend, store: TestBed.inject(AiRuntimeStore) };
}

async function until(cond: () => boolean, rounds = 300): Promise<void> {
  for (let i = 0; i < rounds && !cond(); i++) await settle();
  expect(cond()).toBe(true);
}

describe('AiRuntimeStore', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('loads the overview and offers the suggested build when none is installed', async () => {
    const { store } = setup({ runtime: 'none' });
    expect(store.overview()).toBeNull();
    const o = await store.load();
    expect(o?.needsDownload).toBe(true);
    expect(store.needsDownload()).toBe(true);
    expect(store.suggested()?.id).toBe('cuda-13');
    expect(store.builds()[0]!.id).toBe('cuda-13');
    expect(store.loading()).toBe(false);
  });

  it('goes through downloading, unpacking with byte progress and the runtime check before ready', async () => {
    const { store, backend } = setup({ runtime: 'none' });
    await store.load();
    const stages: string[] = [];
    const unpacked: number[] = [];
    await backend.onRuntime((e) => {
      if (stages[stages.length - 1] !== e.stage) stages.push(e.stage);
      if (e.stage === 'unpacking') unpacked.push(e.done);
    });
    await store.install('cpu');
    await until(() => store.outcome()?.stage === 'ready');
    expect(stages).toEqual(['downloading', 'progress', 'unpacking', 'finishing', 'testing', 'ready']);
    expect(unpacked.length).toBeGreaterThan(2);
    expect(unpacked).toEqual([...unpacked].sort((a, b) => a - b));
  });

  it('installs a build with streamed progress, then reloads', async () => {
    const { store } = setup({ runtime: 'none' });
    await store.load();
    await store.install('cpu');
    expect(store.installing()).toBe(true);
    // A second install while one runs is ignored.
    await store.install('vulkan');
    await until(() => store.job()?.stage?.stage === 'progress');
    expect(store.job()?.id).toBe('cpu');
    await until(() => store.outcome()?.stage === 'ready');
    expect(store.installing()).toBe(false);
    await until(() => !store.needsDownload());
    expect(store.overview()!.builds.find((b) => b.id === 'cpu')).toMatchObject({
      installed: true,
      usable: true,
      active: true,
    });
    store.dismissOutcome();
    expect(store.outcome()).toBeNull();
  });

  it('reports failed and cancelled downloads', async () => {
    const { store } = setup({ runtime: 'none', fail: ['runtime'] });
    await store.load();
    await store.install('vulkan');
    await until(() => !!store.outcome());
    expect(store.outcome()).toMatchObject({ stage: 'failed', id: 'vulkan', code: 'network' });

    TestBed.resetTestingModule();
    const s2 = setup({ runtime: 'none', stepMs: 5 } as MockAiOptions).store;
    await s2.install('cuda-13');
    await until(() => !!s2.job()?.stage);
    await s2.cancel();
    expect(s2.job()?.cancelling).toBe(true);
    await until(() => s2.outcome()?.stage === 'cancelled');
    await s2.cancel(); // nothing runs: no-op
  });

  it('a rejected start ends the install at once', async () => {
    const { store, backend } = setup();
    vi.spyOn(backend, 'runtimeInstall').mockRejectedValue(
      new AiError('busy', 'already downloading'),
    );
    await store.install('cpu');
    expect(store.installing()).toBe(false);
    expect(store.outcome()).toMatchObject({
      stage: 'failed',
      code: 'busy',
      message: 'already downloading',
    });
  });

  it('chooses and deletes builds, keeping errors for the view', async () => {
    const { store } = setup();
    await store.load();
    expect(store.overview()!.active).toBe('vulkan');
    expect(await store.choose('cuda-13')).toBe(false);
    expect(store.error()).toContain('not installed');
    expect(await store.choose('vulkan')).toBe(true);
    expect(store.overview()!.choice).toBe('vulkan');
    expect(store.error()).toBeNull();
    expect(await store.remove('b10012', 'cpu')).toBe(true);
    expect(store.overview()!.outdated).toEqual([]);
    expect(await store.remove('b10012', 'cpu')).toBe(false);
    expect(store.error()).toContain('not found');
    expect(store.busy()).toBe(false);
  });

  it('keeps a failed listing as an error', async () => {
    const { store, backend } = setup();
    vi.spyOn(backend, 'runtimes').mockRejectedValue(new AiError('unavailable', 'AI is off'));
    expect(await store.load()).toBeNull();
    expect(store.error()).toBe('AI is off');
  });

  it('follows installs started in another window', async () => {
    const { store } = setup();
    store.onEvent({ stage: 'downloading', id: 'rocm', asset: 'a.zip', index: 1, count: 1 });
    expect(store.job()).toMatchObject({ id: 'rocm', cancelling: false });
    store.onEvent({ stage: 'cancelled', id: 'rocm' });
    expect(store.installing()).toBe(false);
    expect(store.outcome()?.stage).toBe('cancelled');
  });

  it('installs from archives downloaded by hand and reports a refused choice', async () => {
    const { store } = setup({ runtime: 'none', net: 'offline' });
    await store.load();
    await store.installFromArchives('cpu', ['/d/wrong.zip']);
    expect(store.job()).toBeNull();
    expect(store.outcome()).toMatchObject({ stage: 'failed', id: 'cpu', code: 'invalid' });
    await store.installFromArchives('cpu', ['/d/llama-b11669-bin-win-cpu-x64.zip']);
    await until(() => store.outcome()?.stage === 'ready');
    // The overview is reloaded after the install ended.
    await until(() => store.overview()?.builds.find((b) => b.id === 'cpu')?.installed === true);
  });
});
