import { TestBed } from '@angular/core/testing';
import { settle, testLogger } from '../../testing/helpers';
import { WorkspaceStore } from '../core/workspace.store';
import { LifecycleBackend } from '../core/tools-backend';
import { mockTools, provideToolsTesting } from '../tools/mock/test-support';
import { UI_STATE_CONTRIBUTIONS } from './ui-state';
import { CoreUiState } from './core-ui-state';
import { AUTOSAVE_MS, SessionService } from './session.service';

async function setup() {
  TestBed.configureTestingModule({ providers: [...provideToolsTesting(), { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => TestBed.inject(CoreUiState).contributions }] });
  const session = TestBed.inject(SessionService);
  const store = TestBed.inject(WorkspaceStore);
  await settle();
  session.start();
  TestBed.tick();
  return { session, store, tools: mockTools() };
}

const saves = (tools: { calls: string[] }) => tools.calls.filter((c) => c === 'saveSession').length;

describe('SessionService', () => {
  afterEach(() => vi.useRealTimers());

  it('does not save the startup state, and saves 10 s after a change', async () => {
    const { store, tools } = await setup();
    vi.useFakeTimers();
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MS * 2);
    expect(saves(tools)).toBe(0);
    store.patchQuery({ text: 'x' }); store.setTextInput('x');
    TestBed.tick();
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MS - 1);
    expect(saves(tools)).toBe(0);
    store.patchQuery({ text: 'xy' }); // further changes do not postpone it store.setTextInput('xy');
    TestBed.tick();
    await vi.advanceTimersByTimeAsync(1);
    expect(saves(tools)).toBe(1);
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MS * 3);
    expect(saves(tools)).toBe(1);
  });

  it('saves the latest state when the window loses focus, but only if something changed', async () => {
    const { store, tools, session } = await setup();
    window.dispatchEvent(new Event('blur'));
    await settle();
    expect(saves(tools)).toBe(0);
    store.patchQuery({ text: 'blur' }); store.setTextInput('blur');
    TestBed.tick();
    window.dispatchEvent(new Event('blur'));
    await settle();
    expect(saves(tools)).toBe(1);
    expect(session.lastSaved()).not.toBeNull();
    expect(await session.flush()).toBe(false);
  });

  it('writes nothing while disabled, then picks up changes once enabled', async () => {
    const { store, tools, session } = await setup();
    session.enabled.set(false);
    store.patchQuery({ text: 'held' }); store.setTextInput('held');
    TestBed.tick();
    expect(await session.flush()).toBe(false);
    session.enabled.set(true);
    TestBed.tick();
    expect(await session.flush()).toBe(true);
    expect(saves(tools)).toBe(1);
  });

  it('start is idempotent and flush before start does nothing', async () => {
    TestBed.configureTestingModule({ providers: provideToolsTesting() });
    const session = TestBed.inject(SessionService);
    expect(await session.flush()).toBe(false);
    session.start();
    session.start();
    TestBed.tick();
    expect(await session.flush()).toBe(false);
  });

  it('saveNow writes unconditionally and reports failure without throwing', async () => {
    const { session, tools } = await setup();
    expect(await session.saveNow()).toBe(true);
    expect(await session.saveNow()).toBe(true);
    expect(saves(tools)).toBe(2);
    vi.spyOn(TestBed.inject(LifecycleBackend), 'saveSession').mockRejectedValue(new Error('disk full'));
    expect(await session.saveNow()).toBe(false);
    vi.spyOn(TestBed.inject(LifecycleBackend), 'saveSession').mockRejectedValue('text failure');
    expect(await session.saveNow()).toBe(false);
    expect(testLogger().entries.filter((e) => e.message === 'Session autosave failed')).toHaveLength(2);
  });
});
