import { describe, expect, it, vi } from 'vitest';
import { ChatEventEnvelope, SetupEvent } from '../../ai/ai.types';
import { AI_FAILABLE, MockAiBackend, mockAiOptionsFromSearch } from './mock-ai-backend';
import { demoRecordIds, MOCK_CATALOG } from './mock-ai-data';
import { at } from '../../../testing/must';

const code = (p: Promise<unknown>) =>
  p.then(
    () => null,
    (e) => MockAiBackend.codeOf(e),
  );

function chatOf(b: MockAiBackend) {
  const events: ChatEventEnvelope[] = [];
  void b.onChat((e) => events.push(e));
  return events;
}

async function turn(b: MockAiBackend, events: ChatEventEnvelope[], text: string, id: string) {
  await b.chatSend(id, text, { selectedIds: [] });
  await vi.waitFor(() => expect(events.at(-1)?.type).toMatch(/done|error/));
  return events.splice(0);
}

describe('mockAiOptionsFromSearch', () => {
  it('reads the browser switches', () => {
    expect(mockAiOptionsFromSearch('')).toEqual({});
    expect(
      mockAiOptionsFromSearch(
        '?mockAi=setup&mockAiStepMs=5&mockAiFail=chat,bogus,setup',
      ),
    ).toEqual({ mode: 'setup', stepMs: 5, fail: ['chat', 'setup'] });
    expect(mockAiOptionsFromSearch('?mockAi=weird&mockAiStepMs=x')).toEqual({});
    expect(mockAiOptionsFromSearch('?mockAiRuntime=none&mockAiFail=runtime')).toEqual({
      runtime: 'none',
      fail: ['runtime'],
    });
    expect(mockAiOptionsFromSearch('?mockAiRuntime=broken').runtime).toBe('broken');
    expect(mockAiOptionsFromSearch('?mockAiRuntime=odd')).toEqual({});
    expect(AI_FAILABLE).toContain('propose');
    expect(mockAiOptionsFromSearch('?mockAiNet=tls').net).toBe('tls');
    expect(mockAiOptionsFromSearch('?mockAiNet=proxyAuth').net).toBe('proxyAuth');
    expect(mockAiOptionsFromSearch('?mockAiNet=evil')).toEqual({});
  });
});

describe('MockAiBackend runtimes', () => {
  it('validates deletes and choices like the engine', async () => {
    const b = new MockAiBackend({ stepMs: 0 });
    expect(await code(b.runtimeDelete('..', 'cpu'))).toBe('invalid');
    expect(await code(b.runtimeDelete('b11669', 'cuda-13'))).toBe('notFound');
    expect(await code(b.runtimeChoose('rocm'))).toBe('notFound');
    expect(await code(b.runtimeInstall('metal'))).toBe('notFound');
    expect(await b.runtimeCancel()).toBe(false);
    await b.runtimeInstall('rocm');
    expect(await code(b.runtimeInstall('cpu'))).toBe('busy');
    expect(await code(b.runtimeDelete('b11669', 'rocm'))).toBe('busy');
    const done = new Promise<void>((r) => void b.onRuntime((e) => e.stage === 'ready' && r()));
    await done;
    const rocm = (await b.runtimes()).builds.find((x) => x.id === 'rocm')!;
    expect(rocm).toMatchObject({ installed: true, usable: false });
    await b.runtimeChoose('rocm');
    expect((await b.runtimes()).active).toBe('vulkan');
    await b.runtimeDelete('b11669', 'vulkan');
    expect((await b.status()).backend).toBeNull();
    expect((await b.runtimes()).active).toBeNull();
  });
});

describe('MockAiBackend', () => {
  it('is unavailable when off', async () => {
    const b = new MockAiBackend({ mode: 'off', stepMs: 0 });
    expect((await b.status()).state).toBe('unavailable');
    const calls = [
      () => b.hardware(),
      () => b.recommend(),
      () => b.catalog(),
      () => b.setupPlan(),
      () => b.setupRun('x'),
      () => b.models(),
      () => b.chatNew(),
      () => b.chatSessions(),
      () => b.suggestedPrompts(),
      () => b.nlToQuery('q'),
    ];
    for (const call of calls) expect(await code(call())).toBe('unavailable');
  });

  it('sets up a model with progress, refuses a second set-up and can be cancelled or fail', async () => {
    const b = new MockAiBackend({ mode: 'setup', stepMs: 0 });
    const stages: SetupEvent[] = [];
    const statuses: string[] = [];
    void b.onSetup((e) => stages.push(e));
    void b.onStatus((s) => statuses.push(s.state));
    expect((await b.status()).state).toBe('noModel');
    expect((await b.hardware()).gpus[0]?.vramGb).toBe(8);
    expect((await b.recommend()).best?.model.id).toBe(MOCK_CATALOG[0]?.id);
    expect((await b.catalog()).find((r) => r.model.id === 'qwen3.5-32b-q4km')?.assessment.fit).toBe(
      'tooLarge',
    );
    const plan = await b.setupPlan();
    expect([plan.alreadyInstalled, plan.downloadBytes > 0]).toEqual([false, true]);
    expect((await b.setupPlan('llama3.2-3b-q4km')).chosen?.model.id).toBe('llama3.2-3b-q4km');
    expect(await code(b.setupPlan('nope'))).toBe('notFound');
    expect(await code(b.setupRun('nope'))).toBe('notFound');
    await b.setupRun('qwen3.5-4b-q4km');
    expect(await code(b.setupRun('qwen3.5-4b-q4km'))).toBe('busy');
    expect(await code(b.activateModel('x.gguf'))).toBe('notFound');
    await vi.waitFor(() => expect(stages.at(-1)?.stage).toBe('ready'));
    expect(stages.map((s) => s.stage)).toContain('verifying');
    expect(statuses.at(-1)).toBe('ready');
    expect((await b.setupPlan('qwen3.5-4b-q4km')).alreadyInstalled).toBe(true);
    // Cancel.
    stages.length = 0;
    expect(await b.setupCancel()).toBe(false);
    await b.setupRun('qwen3.5-9b-q4km');
    expect(await code(b.activateModel(at(MOCK_CATALOG, 1).file))).toBe('busy');
    expect(await b.setupCancel()).toBe(true);
    await vi.waitFor(() => expect(stages.at(-1)?.stage).toBe('cancelled'));
    // Re-activating an installed model skips the download.
    stages.length = 0;
    await b.activateModel(at(MOCK_CATALOG, 1).file);
    await vi.waitFor(() => expect(stages.at(-1)?.stage).toBe('ready'));
    expect(stages.some((s) => s.stage === 'downloading')).toBe(false);
    const failing = new MockAiBackend({ mode: 'setup', stepMs: 0, fail: ['setup'] });
    const f: SetupEvent[] = [];
    void failing.onSetup((e) => f.push(e));
    await failing.setupRun('qwen3.5-4b-q4km');
    await vi.waitFor(() => expect(f.at(-1)?.stage).toBe('failed'));
    expect(f.at(-1)).toMatchObject({ code: 'network' });
  });

  it('manages installed models and opens only https links', async () => {
    const b = new MockAiBackend({ stepMs: 0 });
    expect((await b.models())[0]?.active).toBe(true);
    expect(await code(b.addLocalModel('/x.bin'))).toBe('invalid');
    const added = await b.addLocalModel('C:\\models\\My Model.gguf');
    expect(added.fileName).toBe('My_Model.gguf');
    await b.addLocalModel('/again/My Model.gguf');
    expect((await b.models()).length).toBe(2);
    await b.deleteModel('My_Model.gguf');
    await b.deleteModel(at(MOCK_CATALOG, 1).file);
    expect((await b.status()).state).toBe('noModel');
    expect(await code(b.deleteModel('none.gguf'))).toBe('notFound');
    expect((await b.reset()).state).toBe('noModel');
    expect(await code(new MockAiBackend({ stepMs: 0, fail: ['models'] }).models())).toBe('engine');
    expect(await code(b.openLink('http://x'))).toBe('refused');
    await b.openLink('https://x');
    expect(b.opened).toEqual(['https://x']);
    const unsub = await b.onStatus(() => undefined);
    unsub();
  });

  it('answers the use cases', async () => {
    const b = new MockAiBackend({ stepMs: 0 });
    expect((await b.proposeRegex(['2026-10-14 09:00:02 [INF] x'])).matchCount).toBe(1);
    expect(await code(b.proposeRegex([]))).toBe('invalid');
    expect((await b.proposeProfile({})).kind).toBe('profile');
    expect((await b.proposePipeline('g')).rationale).toContain('"g"');
    expect((await b.proposeMarkers('g')).markers).toHaveLength(1);
    expect((await b.proposeExtractors('g')).extractors).toHaveLength(1);
    expect((await b.nlToQuery('show warnings')).ql).toBe('level = warning');
    expect((await b.nlToQuery('errors on web02')).ql).toContain('WEB02');
    expect((await b.nlToQuery('errors')).ql).toBe('level >= error');
    expect((await b.explainEvent(5)).citations[0]).toEqual({
      kind: 'record',
      id: 5,
      verified: true,
    });
    expect((await b.summarize(0, 60_000)).text).toContain('00:00');
    expect((await b.rootCauses()).citations).toHaveLength(3);
    expect(await b.cancel()).toBe(0);
    const ids = demoRecordIds();
    expect(demoRecordIds()).toBe(ids);
    const none = new MockAiBackend({ mode: 'setup', stepMs: 0 });
    expect(await code(none.explainEvent(1))).toBe('noModel');
    const failing = new MockAiBackend({
      stepMs: 0,
      fail: ['propose', 'nlToQuery', 'explain', 'summarize', 'rootCauses'],
    });
    for (const p of [
      failing.proposeProfile({}),
      failing.nlToQuery('q'),
      failing.explainEvent(1),
      failing.summarize(0, 1),
      failing.rootCauses(),
    ])
      expect(await code(p)).toBe('engine');
  });

  it('streams the scripted chat answers', async () => {
    const b = new MockAiBackend({ stepMs: 0 });
    const events = chatOf(b);
    expect(await code(b.chatGet('x'))).toBe('notFound');
    const { id } = await b.chatNew();
    expect(await code(b.chatSend(id, '  ', { selectedIds: [] }))).toBe('invalid');
    expect(await code(b.chatSend('x', 'q', { selectedIds: [] }))).toBe('notFound');
    const types = (ev: ChatEventEnvelope[]) => ev.map((e) => e.type);
    let ev = await turn(b, events, 'what crashed?', id);
    expect(types(ev).slice(0, 2)).toEqual(['toolStarted', 'toolFinished']);
    expect(ev.at(-1)?.type).toBe('done');
    for (const [q, kind] of [
      ['write a regex', 'regex'],
      ['drop these with a pipeline', 'pipeline'],
      ['add a marker', 'markers'],
      ['extract the duration', 'extractors'],
      ['give me a query', 'query'],
    ] as [string, string][]) {
      ev = await turn(b, events, q, id);
      const p = ev.find((e) => e.type === 'proposal');
      expect(p && p.type === 'proposal' ? p.proposal.kind : null).toBe(kind);
    }
    ev = await turn(b, events, 'anything about kubernetes?', id);
    expect(ev.at(-1)).toMatchObject({
      type: 'done',
      text: expect.stringContaining('Not found in the logs'),
    });
    expect(b.sent).toHaveLength(7);
    expect((await b.chatSessions())[0]).toMatchObject({
      id,
      turns: 7,
      running: false,
      title: 'what crashed?',
    });
    expect((await b.chatEventsSince(id, 0))!.length).toBeGreaterThan(10);
    expect(await b.chatEventsSince('none', 0)).toBeNull();
    expect((await b.chatGet(id)).turns).toHaveLength(7);
    expect(
      await code(
        new MockAiBackend({ stepMs: 0, fail: ['chat'] }).chatSend(id, 'q', { selectedIds: [] }),
      ),
    ).toBe('engine');
  });

  it('cancels while streaming and refuses a second turn', async () => {
    const b = new MockAiBackend({ stepMs: 5 });
    const events = chatOf(b);
    const { id } = await b.chatNew();
    await b.chatSend(id, 'what crashed?', { selectedIds: [] });
    expect(await code(b.chatSend(id, 'again', { selectedIds: [] }))).toBe('busy');
    await vi.waitFor(() => expect(events.some((e) => e.type === 'delta')).toBe(true));
    expect(await b.chatCancel(id)).toBe(true);
    await vi.waitFor(() => expect(events.at(-1)).toMatchObject({ type: 'error', cancelled: true }));
    expect((await b.chatGet(id)).turns[0]?.cancelled).toBe(true);
    expect(await b.chatCancel(id)).toBe(false);
    // Cancelled before anything streamed.
    await b.chatSend(id, 'write a regex', { selectedIds: [] });
    await b.chatCancel(id);
    await vi.waitFor(() =>
      expect((async () => (await b.chatSessions())[0]?.running)()).resolves.toBe(false),
    );
  });

  it('saves, restores and exports chats', async () => {
    const b = new MockAiBackend({ stepMs: 0 });
    const events = chatOf(b);
    const { id } = await b.chatNew();
    await turn(b, events, 'q', id);
    const h = await b.chatHistory();
    expect(h.sessions).toHaveLength(1);
    expect(await b.chatExportMarkdown(id, '/a.md')).toBe('/a.md');
    expect(await code(b.chatExportMarkdown(id, '/a.md'))).toBe('fileExists');
    expect(await b.chatExportMarkdown(id, '/a.md', true)).toBe('/a.md');
    expect(await code(b.chatExportMarkdown('none', '/b.md'))).toBe('notFound');
    const fresh = new MockAiBackend({ stepMs: 0 });
    expect(
      (
        await fresh.chatRestore({
          ...h,
          sessions: [...h.sessions, { ...h.sessions[0], id: 'custom' }],
        })
      ).sessions.map((s) => s.id),
    ).toEqual([id, 'custom']);
    expect((await fresh.chatNew()).id).toBe('chat-2');
    expect(await fresh.chatRestore(null)).toEqual({ sessions: [], warning: null });
    expect((await fresh.chatRestore('junk')).warning).toContain('damaged');
    expect((await fresh.chatRestore({ version: 2, sessions: [] })).warning).toContain('newer');
    expect((await fresh.suggestedPrompts()).length).toBeGreaterThan(2);
  });
});

describe('MockAiBackend manual downloads', () => {
  it('fails downloads like a corporate network and names the files to get by hand', async () => {
    const b = new MockAiBackend({ stepMs: 0, mode: 'setup', net: 'tls' });
    const events: { stage: string; code?: string; message?: string }[] = [];
    void b.onSetup((e) => events.push(e as never));
    await b.setupRun('qwen3.5-4b-q4km');
    await vi.waitFor(() => expect(events.at(-1)?.stage).toBe('failed'));
    expect(events.at(-1)).toMatchObject({ code: 'tls' });
    expect(events.at(-1)!.message).toContain('Zscaler');
    const m = await b.manualDownload('model', 'qwen3.5-4b-q4km');
    expect(m.files[0]!.url).toBe('https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/main/Qwen3.5-4B-Q4_K_M.gguf?download=true');
    expect(m.folder).toContain('models');
    const r = await b.manualDownload('runtime', 'cuda-12');
    expect(r.files.map((f) => f.name)).toEqual(['llama-b11669-bin-win-cuda-12-x64.zip', 'cudart-llama-bin-win-cuda-12-x64.zip']);
    expect(r.files.every((f) => /^[0-9a-f]{64}$/.test(f.sha256!))).toBe(true);
    expect(r.name).toBe('CUDA 12 (NVIDIA)');
    expect(await code(b.manualDownload('model', 'nope'))).toBe('notFound');
    expect(await code(b.manualDownload('runtime', 'nope'))).toBe('notFound');
    await b.openManualDownload('runtime', 'cuda-12', 1);
    expect(b.opened.at(-1)).toContain('cudart');
    expect(await code(b.openManualDownload('model', 'qwen3.5-4b-q4km', 5))).toBe('invalid');
    expect(await code(b.runtimeAddArchives('cuda-12', ['/d/llama-b11669-bin-win-cuda-12-x64.zip']))).toBe('invalid');
    // The catalog model downloaded by hand is recognised.
    expect((await b.addLocalModel('/d/Qwen3.5-4B-Q4_K_M.gguf')).catalogId).toBe('qwen3.5-4b-q4km');
  });

  it('classifies every simulated network failure', async () => {
    for (const net of ['proxyAuth', 'proxy', 'offline', 'timeout', 'httpStatus'] as const) {
      const b = new MockAiBackend({ stepMs: 0, mode: 'setup', runtime: 'none', net });
      const events: { stage: string; code?: string }[] = [];
      void b.onRuntime((e) => events.push(e as never));
      await b.runtimeInstall('cpu');
      await vi.waitFor(() => expect(events.at(-1)?.stage).toBe('failed'));
      expect(events.at(-1)!.code).toBe(net);
    }
  });
});
