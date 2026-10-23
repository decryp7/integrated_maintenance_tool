import { TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';
import { flattenContributions, DEBRIEF_COMMANDS, DEBRIEF_VIEWS, DebriefCommand, DebriefView } from '../extensions/tokens';
import { CapturesStore } from './captures.store';
import { PROFILING_VIEW_IDS as V } from './profiling-navigator';
import { ProfileViewState } from './profile-view-state.store';
import { ProfilingCommands, PROFILING_VIEWS, provideProfiling, showOpened, startProfiling } from './provide-profiling';
import { DotnetCommands } from './dotnet/dotnet-commands';
import { NativeCommands } from './native/native-commands';
import { provideProfilingTesting, setupProfiling } from './testing/profiling-test-support';
import { FakeContext } from '../analysis/test-setup';
import { settle } from '../../testing/helpers';
import { MOCK_CAPTURE_PATHS } from '../core/mock/mock-profiling-data';
import { DROPPED_FILES_HANDLERS } from '../core/dropped-files';
import { UI_STATE_CONTRIBUTIONS, UiStateContribution } from '../lifecycle/ui-state';
import { PROFILING_STATE_KEY } from './profiling-persistence';
import { at } from '../../testing/must';

const ORDERS = '/demo/profiles/orders.speedscope.json';
const HEAP1 = '/demo/profiles/heap-1.heapsnapshot';

const shown = (ctx: FakeContext) => ctx.calls.filter((c) => c.fn === 'showView').map((c) => c.args[0]);

describe('PROFILING_VIEWS', () => {
  it('has unique ids, one per profiling view id', () => {
    const ids = PROFILING_VIEWS.map((v) => v.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect([...ids].sort()).toEqual(Object.values(V).sort());
  });

  it('every view has a title, an icon and a loader that resolves to a component class', async () => {
    for (const view of PROFILING_VIEWS) {
      expect(view.title).toBeTruthy();
      expect(view.icon).toBeTruthy();
      const component = await view.component();
      expect(typeof component, view.id).toBe('function');
    }
  });
});

describe('ProfilingCommands', () => {
  function setup() {
    const doubles = setupProfiling();
    return { commands: TestBed.inject(ProfilingCommands), ctx: doubles.context, picker: doubles.picker, captures: TestBed.inject(CapturesStore) };
  }
  const byId = (commands: ProfilingCommands, id: string): DebriefCommand => commands.commands.find((c) => c.id === id)!;

  it('has unique command ids; only "profiling.open" is in the File category', () => {
    const { commands } = setup();
    const ids = commands.commands.map((c) => c.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(byId(commands, 'profiling.open').category).toBe('File');
    expect(commands.commands.filter((c) => c.id !== 'profiling.open').every((c) => c.category === 'Profiling')).toBe(true);
  });

  it.each([
    ['profiling.captures', V.captures],
    ['profiling.hotspots', V.hotspots],
    ['profiling.tree', V.tree],
    ['profiling.inverted', V.inverted],
    ['profiling.butterfly', V.butterfly],
    ['profiling.flame', V.flame],
    ['profiling.compare', V.compare],
    ['profiling.threads', V.threads],
    ['profiling.findings', V.findings],
    ['profiling.heap', V.heap],
  ])('%s shows %s', async (id, view) => {
    const { commands, ctx } = setup();
    await byId(commands, id).run(ctx);
    expect(shown(ctx)).toEqual([view]);
  });

  it('every profiling view has a command', () => {
    const { commands } = setup();
    expect(commands.commands).toHaveLength(Object.keys(V).length + 1);
  });

  it('open shows the captures and the hotspots after opening profiles', async () => {
    const { commands, ctx, picker, captures } = setup();
    picker.paths = [ORDERS, HEAP1];
    await byId(commands, 'profiling.open').run(ctx);
    expect(captures.captures()).toHaveLength(2);
    expect(shown(ctx)).toEqual([V.captures, V.hotspots]);
  });

  it('open shows the memory view when only heap snapshots were opened', async () => {
    const { commands, ctx, picker } = setup();
    picker.paths = [HEAP1];
    await commands.open(ctx);
    expect(shown(ctx)).toEqual([V.captures, V.heap]);
  });

  it('open shows the captures list with the error when something failed', async () => {
    const { commands, ctx, picker, captures } = setup();
    picker.paths = ['/demo/readme.txt'];
    await commands.open(ctx);
    expect(captures.error()).toContain('readme.txt');
    expect(shown(ctx)).toEqual([V.captures]);
  });

  it('suggests the Profiling perspective before showing the views, for profiles and heap snapshots alike', async () => {
    const { commands, ctx, picker } = setup();
    picker.paths = [ORDERS];
    await commands.open(ctx);
    expect(ctx.calls.map((c) => c.fn)).toEqual(['suggestPerspective', 'showView', 'showView']);
    expect(ctx.calls[0]?.args).toEqual(['profiling']);
    const heap = new FakeContext();
    picker.paths = [HEAP1];
    await commands.open(heap);
    expect(heap.calls[0]).toEqual({ fn: 'suggestPerspective', args: ['profiling'] });
  });

  it('does not suggest it when the dialog was cancelled or nothing could be opened', async () => {
    const { commands, ctx, picker } = setup();
    picker.paths = [];
    await commands.open(ctx);
    picker.paths = ['/demo/readme.txt'];
    await commands.open(ctx);
    expect(ctx.calls.filter((c) => c.fn === 'suggestPerspective')).toEqual([]);
  });

  it('open does nothing visible when the dialog is cancelled', async () => {
    const { commands, ctx, picker } = setup();
    picker.paths = [];
    await commands.open(ctx);
    expect(ctx.calls).toEqual([]);
  });

  it('open shows the captures (not just the opened views) when some files failed', async () => {
    const { commands, ctx, picker } = setup();
    picker.paths = ['/demo/readme.txt', ORDERS];
    await commands.open(ctx);
    expect(shown(ctx)).toEqual([V.captures, V.hotspots, V.captures]);
  });
});

describe('showOpened', () => {
  it('shows nothing when nothing was opened', () => {
    const ctx = new FakeContext();
    showOpened(ctx, []);
    expect(ctx.calls).toEqual([]);
  });

  it('copes with a context that cannot switch perspectives', () => {
    const calls: unknown[] = [];
    const ctx = { showView: (id: string) => void calls.push(id) } as never;
    showOpened(ctx, [{ id: 1, capture: 'profile' } as never]);
    expect(calls).toEqual([V.captures, V.hotspots]);
  });
});

describe('startProfiling and provideProfiling', () => {
  it('registers the views and commands under the extension tokens', () => {
    const { providers } = provideProfilingTesting();
    TestBed.configureTestingModule({ providers: [provideProfiling(), ...providers] });
    const views = flattenContributions<DebriefView>(TestBed.inject(DEBRIEF_VIEWS));
    const commands = flattenContributions<DebriefCommand>(TestBed.inject(DEBRIEF_COMMANDS));
    expect(views).toEqual(PROFILING_VIEWS);
    expect(commands.map((c) => c.id)).toEqual([...TestBed.inject(ProfilingCommands).commands, ...TestBed.inject(DotnetCommands).commands, ...TestBed.inject(NativeCommands).commands].map((c) => c.id));
  });

  it('refreshes at startup and claims dropped captures, leaving other files to the log import', async () => {
    const { providers, doubles } = provideProfilingTesting({ options: { preopen: [ORDERS] } });
    TestBed.configureTestingModule({ providers: [provideProfiling(), ...providers] });
    TestBed.inject(DEBRIEF_VIEWS);
    await settle();
    const captures = TestBed.inject(CapturesStore);
    expect(captures.profiles()).toHaveLength(1);
    expect(TestBed.inject(ProfileViewState).profileId()).toBe(captures.profiles()[0]?.id);
    expect(doubles.platform.dropped, 'profiling no longer listens to drops itself').toBeNull();
    const handlers = TestBed.inject(DROPPED_FILES_HANDLERS);
    expect(handlers).toHaveLength(1);
    const drop = at(handlers, 0);

    expect(await drop(['/demo/readme.txt'])).toEqual([]);
    expect(doubles.context.calls).toEqual([]);
    expect(captures.captures()).toHaveLength(1);

    expect(await drop([HEAP1, '/demo/readme.txt'])).toEqual([HEAP1]);
    expect(captures.heaps()).toHaveLength(1);
    expect(shown(doubles.context)).toEqual([V.captures, V.heap]);
    expect(captures.error()).toBeNull();

    expect(await drop(MOCK_CAPTURE_PATHS.slice(0, 2))).toEqual(MOCK_CAPTURE_PATHS.slice(0, 2));
    expect(shown(doubles.context).slice(2)).toEqual([V.captures, V.hotspots]);
  });

  it('shows the real error of a dropped capture that fails to open, and still claims it (no log import)', async () => {
    const { providers, doubles } = provideProfilingTesting({ options: { fail: { open: 1 } } });
    TestBed.configureTestingModule({ providers: [provideProfiling(), ...providers] });
    const drop = at(TestBed.inject(DROPPED_FILES_HANDLERS), 0);
    expect(await drop(['/x/orders.nettrace'])).toEqual(['/x/orders.nettrace']);
    const captures = TestBed.inject(CapturesStore);
    expect(captures.error()).toBe('mock: open failed (simulated)');
    expect(captures.failedPaths()).toEqual(['/x/orders.nettrace']);
    expect(shown(doubles.context)).toEqual([V.captures]);
  });

  it('claims nothing when sniffing fails, so the files still open as logs', async () => {
    const { providers } = provideProfilingTesting({ options: { fail: { sniff: 1 } } });
    TestBed.configureTestingModule({ providers: [provideProfiling(), ...providers] });
    const drop = at(TestBed.inject(DROPPED_FILES_HANDLERS), 0);
    await expect(drop(['/x/orders.nettrace'])).rejects.toMatchObject({ code: 'io' });
    expect(TestBed.inject(CapturesStore).captures()).toEqual([]);
  });

  it('saves its state with the project and the session', () => {
    const { providers } = provideProfilingTesting();
    TestBed.configureTestingModule({ providers: [provideProfiling(), ...providers] });
    const keys = flattenContributions<UiStateContribution>(TestBed.inject(UI_STATE_CONTRIBUTIONS)).map((c) => c.key);
    expect(keys).toContain(PROFILING_STATE_KEY);
  });

  it('startProfiling can run in an injection context on its own', async () => {
    setupProfiling({ options: { preopen: [HEAP1] } });
    TestBed.runInInjectionContext(() => startProfiling());
    await settle();
    expect(TestBed.inject(CapturesStore).heaps()).toHaveLength(1);
  });
});
