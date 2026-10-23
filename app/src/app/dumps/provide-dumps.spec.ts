import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { settle } from '../../testing/helpers';
import { DumpDocSummary } from '../core/api.types';
import { DROPPED_FILES_HANDLERS } from '../core/dropped-files';
import { flattenContributions, DEBRIEF_COMMANDS, DEBRIEF_VIEWS, DebriefCommand, DebriefView } from '../extensions/tokens';
import { UI_STATE_CONTRIBUTIONS, UiStateContribution } from '../lifecycle/ui-state';
import { DumpsExplainer, formatBytes, formatDelta } from './dumps-shared';
import { DumpsBackend } from './dumps-backend';
import { DumpsStore } from './dumps.store';
import { MOCK_DUMP_PATHS, MOCK_UMDH_PATHS } from './mock/mock-dumps-backend';
import { DUMPS_STATE_KEY } from './dumps-persistence';
import { DumpsCommands, dumpsViews, provideDumps, showOpened } from './provide-dumps';
import { setupDumps } from './testing/dumps-test-support';
import { COMMAND_CONTEXT } from '../extensions/command-registry';

const shown = (ctx: { calls: { fn: string; args: unknown[] }[] }) => ctx.calls.filter((c) => c.fn === 'showView').map((c) => c.args[0]);

describe('dumps views and commands', () => {
  it('declares three views whose components load', async () => {
    const views = dumpsViews({ listed: () => false });
    expect(views.map((v) => v.id)).toEqual(['dump', 'umdh', 'dump-capture']);
    expect(views[2]!.enabled!()).toBe(false);
    for (const v of views) expect(typeof (await v.component())).toBe('function');
  });

  it('opens dumps and UMDH logs from the File menu and shows their views', async () => {
    const t = setupDumps();
    const commands = TestBed.inject(DumpsCommands);
    const byId = (id: string) => commands.commands.find((c) => c.id === id)!;
    expect(byId('dumps.open').category).toBe('File');
    await byId('dumps.open').run(t.context);
    await byId('dumps.openUmdh').run(t.context);
    expect(shown(t.context)).toEqual(['dump', 'umdh']);
    await byId('dumps.capture').run(t.context);
    await byId('dumps.show').run(t.context);
    await byId('dumps.showUmdh').run(t.context);
    expect(shown(t.context).slice(2)).toEqual(['dump-capture', 'dump', 'umdh']);
    expect(byId('dumps.capture').when!(t.context)).toBe(true);
    // A failed open shows the view with its error.
    t.picker.dumps = ['/x/notes.txt'];
    await commands.open(t.context, 'dump');
    expect(shown(t.context).at(-1)).toBe('dump');
  });

  it('shows the view of what opened', () => {
    const calls: string[] = [];
    const ctx = { showView: (id: string) => calls.push(id) } as never;
    showOpened(ctx, [{ kind: 'umdhSnapshot' } as DumpDocSummary, { kind: 'dump' } as DumpDocSummary]);
    showOpened(ctx, []);
    expect(calls).toEqual(['umdh', 'dump']);
  });

  it('provides the backend, views, commands, drop handler, persistence and the start-up refresh', async () => {
    const t = setupDumps();
    TestBed.configureTestingModule({ providers: [provideDumps(), { provide: DumpsBackend, useValue: t.backend }, { provide: COMMAND_CONTEXT, useValue: t.context }] });
    const views = flattenContributions<DebriefView>(TestBed.inject(DEBRIEF_VIEWS));
    expect(views.map((v) => v.id)).toContain('umdh');
    const commands = flattenContributions<DebriefCommand>(TestBed.inject(DEBRIEF_COMMANDS));
    expect(commands.map((c) => c.id)).toContain('dumps.open');
    const contributions = flattenContributions<UiStateContribution>(TestBed.inject(UI_STATE_CONTRIBUTIONS));
    expect(contributions.map((c) => c.key)).toContain(DUMPS_STATE_KEY);
    const [drop] = TestBed.inject(DROPPED_FILES_HANDLERS);
    const claimed = await drop!([MOCK_DUMP_PATHS[0]!, '/x/app.log', MOCK_UMDH_PATHS[0]!]);
    expect(claimed).toEqual([MOCK_DUMP_PATHS[0], MOCK_UMDH_PATHS[0]]);
    expect(shown(t.context)).toEqual(['umdh', 'dump']);
    // Claimed but not opened: the view shows the error.
    t.backend.open = () => Promise.reject(new Error('damaged'));
    await drop!([MOCK_DUMP_PATHS[0]!]);
    expect(shown(t.context).at(-1)).toBe('dump');
    await settle(undefined, 4);
    expect(TestBed.inject(DumpsStore).docs().length).toBeGreaterThan(0);
  });
});

describe('dumps shared', () => {
  it('formats bytes and changes', () => {
    
    expect(formatBytes(-2048)).toBe('-2 KB');
    expect(formatBytes(3 << 20)).toBe('3 MB');
    expect(formatBytes(2 ** 31)).toBe('2 GB');
    expect(formatDelta(1024)).toBe('+1 KB');
    expect(formatDelta(-1024)).toBe('-1 KB');
  });

  it('sends the engine summary to the chat', async () => {
    const t = setupDumps();
    const send = vi.fn(async () => true);
    const { ChatStore } = await import('../ai/chat/chat.store');
    TestBed.overrideProvider(ChatStore, { useValue: { send } });
    const store = TestBed.inject(DumpsStore);
    await store.pickAndOpen('dump');
    expect(await TestBed.inject(DumpsExplainer).explain([store.dump()!.id])).toBe(true);
    expect(shown(t.context)).toEqual(['ai-chat']);
    expect(send).toHaveBeenCalledWith(expect.stringContaining('OrderService.exe.4120.dmp'));
  });
});
