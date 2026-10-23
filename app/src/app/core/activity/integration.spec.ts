import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { MemoryLogger, provideFakeDock, provideTinyBackendBare, FakeDock } from '../../../testing/helpers';
import { COMMAND_CONTEXT, CommandRegistry } from '../../extensions/command-registry';
import { CommandContext, DEBRIEF_COMMANDS, DebriefCommand } from '../../extensions/tokens';
import { PANEL_WINDOWS, PanelWindows } from '../../dock/panel-windows';
import { DockService } from '../../dock/dock.service';
import { Logger, withFailureLogging } from '../logger';
import { WorkspaceStore } from '../workspace.store';
import { ActivityLog } from './activity-log';
import { methodKind } from './backend-activity';
import { ActivityObservers, queryChanges, QUERY_DEBOUNCE_MS, SELECTION_THROTTLE_MS } from './observers';
import { activity } from './runtime';
import { EMPTY_QUERY } from '../workspace.store';
import { RecordView } from '../api.types';
import { at } from '../../../testing/must';

function entries(): { message: string; props: Record<string, string> }[] {
  return (TestBed.inject(Logger) as MemoryLogger).entries
    .filter((e) => e.category === 'Debrief.Activity')
    .map((e) => ({ message: e.message, props: e.props as Record<string, string> }));
}

const ctx: CommandContext = {
  summary: null,
  openPaths: async () => undefined,
  setQuery: () => undefined,
  addFilter: () => undefined,
  selectRecord: () => undefined,
  jumpTo: async () => undefined,
  showView: () => undefined,
  notifyError: () => undefined,
};

describe('activity: trigger attribution of command entry paths', () => {
  let ran: string[];
  beforeEach(() => {
    activity.reset();
    ran = [];
    document.body.innerHTML = '';
  });
  afterEach(() => (document.body.innerHTML = ''));

  function setup(commands: DebriefCommand[]) {
    TestBed.configureTestingModule({
      providers: [provideTinyBackendBare(), provideFakeDock(), { provide: COMMAND_CONTEXT, useValue: ctx }, { provide: DEBRIEF_COMMANDS, multi: true, useValue: commands }],
    });
    TestBed.inject(ActivityLog);
    const stop = activity.triggers.install(document);
    return { registry: TestBed.inject(CommandRegistry), stop };
  }
  const cmd = (id: string, extra: Partial<DebriefCommand> = {}): DebriefCommand => ({ id, title: `Do ${id}`, category: 'File', run: () => void ran.push(id), ...extra });

  /** Simulates a click on the element matched by `selector` in `html`, then runs the command (as its handler would). */
  async function clickThenRun(registry: CommandRegistry, html: string, selector: string, id: string) {
    document.body.innerHTML = html;
    const el = document.querySelector(selector)!;
    el.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
    el.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }));
    await registry.run(id);
  }

  it('attributes menu, context menu, button, palette, shortcut and native menu runs', async () => {
    const { registry, stop } = setup([cmd('a', { keybinding: 'Mod+Shift+K' })]);
    await clickThenRun(registry, '<debrief-title-bar><div role="menu"><button role="menuitem" id="i">A</button></div></debrief-title-bar>', '#i', 'a');
    await clickThenRun(registry, '<div role="menu"><button role="menuitem" id="i">A</button></div>', '#i', 'a');
    await clickThenRun(registry, '<div><button id="i">A</button></div>', '#i', 'a');
    await registry.run('a', 'palette');
    expect(registry.runForKey(new KeyboardEvent('keydown', { key: 'K', ctrlKey: true, shiftKey: true }))).toBe(true);
    await Promise.resolve();
    await Promise.resolve();
    // A native menu click reaches the webview as an event, with no DOM input.
    activity.triggers.noteNativeMenu('a');
    await registry.run('a');
    stop();
    expect(ran).toHaveLength(6);
    expect(entries().map((e) => e.props['Trigger'])).toEqual(['menu', 'context-menu', 'button', 'palette', 'shortcut', 'native-menu']);
    expect(entries()[0]?.props).toMatchObject({ Operation: 'a', Title: 'Do a', Category: 'File', Outcome: 'ok' });
    expect(entries()[0]?.message).toBe('User a via menu');
  });

  it('logs failed commands with the error outcome and the same operation id as the failure log', async () => {
    const { registry } = setup([
      cmd('bad', {
        run: () => {
          throw Object.assign(new Error('disk full'), { code: 'E_IO' });
        },
      }),
    ]);
    expect(await registry.run('bad', 'menu')).toBe(false);
    const all = (TestBed.inject(Logger) as MemoryLogger).entries;
    const entry = all.find((e) => e.category === 'Debrief.Activity')!;
    const failure = all.find((e) => e.category === 'ui.command')!;
    expect(entry.props).toMatchObject({ Outcome: 'error', ErrorCode: 'E_IO' });
    expect(failure.props?.['Command']).toBe('bad');
    expect(await registry.run('missing')).toBe(false);
    expect(entries()).toHaveLength(1);
  });
});

describe('activity: backend calls', () => {
  beforeEach(() => activity.reset());

  class FakeBackend {
    calls: string[] = [];
    queryPage() {
      this.calls.push('queryPage');
      return Promise.resolve(1);
    }
    exportEvents(_req: unknown) {
      this.calls.push('exportEvents');
      return Promise.resolve({ rows: 3 });
    }
    saveProfile() {
      return Promise.reject(new Error('profile does not compile'));
    }
    importProfile() {
      return Promise.reject(new Error('import cancelled'));
    }
    setLive(on: boolean) {
      return on;
    }
    explode(): never {
      throw new Error('sync failure');
    }
    onEvent() {
      return Promise.resolve(() => undefined);
    }
  }

  function setup() {
    TestBed.configureTestingModule({ providers: [{ provide: Logger, useFactory: () => new MemoryLogger() }] });
    TestBed.inject(ActivityLog);
    return withFailureLogging(new FakeBackend(), TestBed.inject(Logger));
  }

  it('turns a change outside any operation into an operation, with domain names', async () => {
    const b = setup();
    // Without user input a change is not a user operation (start-up, timers, another window).
    await b.exportEvents({});
    expect(entries()).toHaveLength(0);
    activity.triggers.note('dialog');
    await b.queryPage();
    await b.onEvent();
    expect(entries()).toHaveLength(0);
    await b.exportEvents({ format: 'csv', path: '/Users/alice/out.csv' });
    expect(entries()[0]?.props).toMatchObject({ Operation: 'file.export', Format: 'csv', Method: 'exportEvents', Outcome: 'ok', Trigger: 'dialog' });
    expect(entries()[0]?.props['Path']).toMatch(/^<dir#[0-9a-f]{8}>\/out\.csv$/);
    activity.triggers.note('button');
    await expect(b.saveProfile()).rejects.toThrow('compile');
    expect(entries()[1]?.props).toMatchObject({ Operation: 'backend.saveProfile', Outcome: 'error' });
    activity.triggers.note('dialog');
    await expect(b.importProfile()).rejects.toThrow('cancelled');
    expect(entries()[2]?.props['Outcome']).toBe('cancelled');
    activity.triggers.note('button');
    expect(b.setLive(true)).toBe(true);
    expect(entries()[3]?.props['Operation']).toBe('live.on');
    activity.triggers.note('button');
    expect(() => b.explode()).toThrow('sync failure');
    expect(entries()[4]?.props).toMatchObject({ Operation: 'backend.explode', Outcome: 'error' });
  });

  it('attributes calls inside an operation to it, and logs failures with severity and the operation id', async () => {
    const b = setup();
    let id = '';
    await expect(
      activity.track('profile.save', 'menu', {}, async () => {
        id = activity.currentOperationId()!;
        await b.exportEvents({});
        await b.saveProfile();
      }),
    ).rejects.toThrow();
    expect(entries().map((e) => e.props['Operation'])).toEqual(['profile.save']);
    const failure = (TestBed.inject(Logger) as MemoryLogger).entries.find((e) => e.message === 'Backend command failed')!;
    expect(failure.level).toBe('warn');
    expect(failure.props).toMatchObject({ Command: 'saveProfile', ErrorCode: 'E_FAILED' });
    expect(id).toBeTruthy();
  });

  it('classifies methods', () => {
    expect(['queryPage', 'listErrorSnapshots', 'previewParse', 'workspaceSummary', 'profilingTree'].map(methodKind)).toEqual(['query', 'query', 'query', 'query', 'query']);
    expect(['openPaths', 'saveProfile', 'exportEvents', 'profilingOpen', 'previewRedaction', 'set'].map(methodKind)).toEqual(Array(6).fill('mutation'));
    expect(methodKind('saveSession')).toBe('quiet');
    // Event Log follower reads are queries, not user operations; start/stop are.
    for (const m of ['eventLogChannels', 'eventLogValidate', 'eventLogStatus']) expect(methodKind(m)).toBe('query');
    expect(methodKind('eventLogStart')).toBe('mutation');
    expect(methodKind('onWorkspaceChanged')).toBe('event');
  });
});

describe('activity: observers', () => {
  beforeEach(() => {
    activity.reset();
    vi.useFakeTimers();
  });
  afterEach(() => vi.useRealTimers());

  class FakePanelWindows implements Partial<PanelWindows> {
    readonly poppedIds = signal<ReadonlySet<string>>(new Set());
  }

  function setup() {
    const dock = new FakeDock();
    const autoHide = signal({ items: [] as { id: string }[], open: null });
    const windows = new FakePanelWindows();
    TestBed.configureTestingModule({
      providers: [
        provideTinyBackendBare(),
        { provide: DockService, useValue: Object.assign(dock, { autoHide }) },
        { provide: PANEL_WINDOWS, useValue: windows },
      ],
    });
    TestBed.inject(ActivityObservers).start();
    TestBed.inject(ActivityObservers).start();
    TestBed.tick();
    return { store: TestBed.inject(WorkspaceStore), dock, autoHide, windows };
  }
  const userInput = () => activity.triggers.note('keyboard');
  const settle = (ms: number) => {
    TestBed.tick();
    vi.advanceTimersByTime(ms);
  };

  it('debounces query typing into one entry without the text', () => {
    const { store } = setup();
    for (const ql of ['l', 'le', 'level:error']) {
      userInput();
      store.patchQuery({ ql });
      TestBed.tick();
      vi.advanceTimersByTime(100);
    }
    expect(entries()).toHaveLength(0);
    settle(QUERY_DEBOUNCE_MS);
    expect(entries()).toHaveLength(1);
    const e = at(entries(), 0).props;
    expect(e).toMatchObject({ Operation: 'query.change', Trigger: 'keyboard', Count: '3', QueryLength: '11' });
    expect(JSON.stringify(entries())).not.toContain('level:error');
  });

  it('debounces time-range changes and logs search and filter changes', () => {
    const { store } = setup();
    userInput();
    store.patchQuery({ from: 1, to: 1000 });
    TestBed.tick();
    vi.advanceTimersByTime(200);
    store.patchQuery({ from: 1, to: 2000 });
    settle(QUERY_DEBOUNCE_MS);
    userInput();
    store.patchQuery({ text: 'secret words', levels: ['Error'] });
    settle(QUERY_DEBOUNCE_MS);
    const ops = entries().map((e) => e.props['Operation']);
    expect(ops).toEqual(['timeRange.change', 'search.change', 'filter.change']);
    expect(entries()[0]?.props).toMatchObject({ RangeMs: '1999', Count: '2' });
    expect(entries()[2]?.props['Levels']).toBe('Error');
    expect(JSON.stringify(entries())).not.toContain('secret words');
  });

  it('ignores programmatic changes and changes made by a running command', async () => {
    const { store } = setup();
    vi.advanceTimersByTime(5000);
    store.patchQuery({ ql: 'restored' });
    settle(QUERY_DEBOUNCE_MS);
    expect(entries()).toHaveLength(0);
    await activity.track('analyze.clearFilters', 'menu', {}, () => {
      store.patchQuery({ ql: '' });
      TestBed.tick();
    });
    settle(QUERY_DEBOUNCE_MS);
    expect(entries().map((e) => e.props['Operation'])).toEqual(['analyze.clearFilters']);
  });

  it('throttles selection to ids only', () => {
    const { store } = setup();
    for (const id of [1, 2, 3]) {
      userInput();
      store.select({ id, message: `secret message ${id}` } as unknown as RecordView);
      TestBed.tick();
      vi.advanceTimersByTime(50);
    }
    settle(SELECTION_THROTTLE_MS);
    expect(entries()).toHaveLength(1);
    expect(entries()[0]?.props).toMatchObject({ Operation: 'selection.change', RecordId: '3', Count: '3' });
    expect(JSON.stringify(entries())).not.toContain('secret message');
  });

  it('logs dock operations', () => {
    const { dock, autoHide, windows } = setup();
    activity.triggers.note('button');
    dock.openIds.set(new Set(['timeline', 'inspector']));
    TestBed.tick();
    activity.triggers.note('button');
    windows.poppedIds.set(new Set(['inspector']));
    TestBed.tick();
    activity.triggers.note('context-menu');
    autoHide.set({ items: [{ id: 'timeline' }], open: null });
    TestBed.tick();
    activity.triggers.note('button');
    windows.poppedIds.set(new Set());
    dock.openIds.set(new Set(['timeline']));
    TestBed.tick();
    const ops = entries().map((e) => `${e.props['Operation']}:${e.props['Panel']}:${e.props['Trigger']}`);
    expect(ops).toEqual(['dock.open:inspector:button', 'dock.popout:inspector:button', 'dock.autoHide:timeline:context-menu', 'dock.close:inspector:button', 'dock.dockBack:inspector:button']);
  });

  it('describes query changes', () => {
    const q = EMPTY_QUERY;
    expect(queryChanges(q, q)).toEqual([]);
    expect(queryChanges(q, { ...q, regex: true, ql: 'x', groups: ['a'], window: { kind: 'live', lastMs: 5 } })).toEqual(['search', 'query', 'filter', 'timeRange']);
  });
});
