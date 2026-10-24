import { TestBed } from '@angular/core/testing';
import { settle } from '../../testing/helpers';
import { WorkspaceStore } from '../core/workspace.store';
import { StartupBackend } from '../core/startup/startup-backend';
import { AFTER_READY } from '../core/startup/startup-contract';
import { provideToolsTesting } from '../tools/mock/test-support';
import { CoreUiState } from './core-ui-state';
import { CHECKPOINT_MS, provideRecovery, recoveryNotice, WindowRecovery } from './recovery';
import { UI_STATE_CONTRIBUTIONS } from './ui-state';

class FakeStartup implements StartupBackend {
  readonly checkpoints: unknown[] = [];
  async reportReady() {
    return { recovered: null };
  }
  async checkpointUi(ui: unknown) {
    this.checkpoints.push(ui);
  }
}

async function setup() {
  const startup = new FakeStartup();
  TestBed.configureTestingModule({
    providers: [
      ...provideToolsTesting(),
      { provide: StartupBackend, useValue: startup },
      { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => TestBed.inject(CoreUiState).contributions },
      provideRecovery(),
    ],
  });
  const store = TestBed.inject(WorkspaceStore);
  const recovery = TestBed.inject(WindowRecovery);
  await settle();
  return { store, recovery, startup };
}

const lastQuery = (s: FakeStartup) => (s.checkpoints.at(-1) as { query?: { textInput?: string } } | undefined)?.query?.textInput;

describe('WindowRecovery', () => {
  afterEach(() => vi.useRealTimers());

  it('a recovery reload restores the checkpointed query and selection once the workspace is loaded, and says so', async () => {
    const { store, recovery } = await setup();
    await vi.waitFor(() => expect(store.summary()).not.toBeNull());
    const id = 3;
    await recovery.afterReady({
      recovered: 'terminated',
      ui: { query: { query: { text: '', levels: ['error'] }, textInput: '' }, selection: { recordId: id } },
    });
    expect(store.query().levels).toEqual(['error']);
    expect(store.anchor()?.id).toBe(id); // the grid resolves the anchor into the selection
    expect(store.notice()).toBe(recoveryNotice('terminated'));
    await recovery.afterReady({ recovered: 'unresponsive', ui: { query: { query: { text: 'timeout' }, textInput: 'timeout' } } });
    expect(store.textInput()).toBe('timeout');
    expect(store.query().text).toBe('timeout');
    expect(store.notice()).toBe(recoveryNotice('unresponsive'));
  });

  it('a normal start restores nothing and shows no notice', async () => {
    const { store, recovery } = await setup();
    await recovery.afterReady({ recovered: null, ui: { query: { textInput: 'stale' } } });
    expect(store.textInput()).toBe('');
    expect(store.notice()).toBeNull();
  });

  it('checkpoints the UI state after a quiet second, only once the reply was handled', async () => {
    const { store, recovery, startup } = await setup();
    vi.useFakeTimers();
    store.setTextInput('before ready');
    TestBed.tick();
    await vi.advanceTimersByTimeAsync(CHECKPOINT_MS * 3);
    expect(startup.checkpoints).toEqual([]);
    await recovery.afterReady(null);
    TestBed.tick();
    await vi.advanceTimersByTimeAsync(CHECKPOINT_MS);
    store.setTextInput('a');
    TestBed.tick();
    await vi.advanceTimersByTimeAsync(CHECKPOINT_MS - 1);
    store.setTextInput('ab');
    TestBed.tick();
    await vi.advanceTimersByTimeAsync(CHECKPOINT_MS);
    expect(lastQuery(startup)).toBe('ab');
    const n = startup.checkpoints.length;
    await vi.advanceTimersByTimeAsync(CHECKPOINT_MS * 5);
    expect(startup.checkpoints.length).toBe(n);
  });

  it('the main window handles the reply through AFTER_READY', async () => {
    await setup();
    const spy = vi.spyOn(TestBed.inject(WindowRecovery), 'afterReady');
    TestBed.inject(AFTER_READY)({ recovered: null });
    expect(spy).toHaveBeenCalledWith({ recovered: null });
  });
});
