import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, settle } from '../../testing/helpers';
import { EMPTY_QUERY, WorkspaceStore } from '../core/workspace.store';
import { ImtBackend } from '../core/backend';
import { GridSelection } from '../grid/grid-selection';
import { EditCommands, eventsJson } from './edit-commands';

describe('Edit menu commands (F4)', () => {
  let written: string[];
  beforeEach(() => {
    written = [];
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async (t: string) => void written.push(t) } });
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
  });

  const run = (id: string) => TestBed.inject(EditCommands).commands.find((c) => c.id === id)!.run({} as never);

  it('copies the selected events (messages, or JSON), and says when nothing is selected', async () => {
    const store = TestBed.inject(WorkspaceStore);
    await settle();
    const rows = (await TestBed.inject(ImtBackend).queryPage({ query: EMPTY_QUERY, offset: 0, limit: 2, descending: false })).rows;
    window.getSelection()?.removeAllRanges();
    await run('edit.copy');
    expect(written).toEqual([]);
    expect(store.notice()).toBe('Select events or text first.');
    await run('edit.copyJson');
    expect(written).toEqual([]);

    store.select(rows[0]!);
    await run('edit.copy');
    expect(written.at(-1)).toBe(rows[0]!.message);
    await run('edit.copyJson');
    expect(JSON.parse(written.at(-1)!)).toMatchObject({ id: rows[0]!.id });

    TestBed.inject(GridSelection).publish(rows);
    await run('edit.copy');
    expect(written.at(-1)).toBe(rows.map((r) => r.message).join('\n'));
    await run('edit.copyJson');
    expect(JSON.parse(written.at(-1)!)).toHaveLength(2);
  });

  it('Find (Ctrl+F) focuses the search box', async () => {
    const store = TestBed.inject(WorkspaceStore);
    const before = store.focusSearchNonce();
    await run('edit.find');
    expect(store.focusSearchNonce()).toBe(before + 1);
    expect(TestBed.inject(EditCommands).commands.find((c) => c.id === 'edit.find')?.keybinding).toBe('Mod+F');
  });

  it('formats one event as an object and several as an array', () => {
    expect(eventsJson([])).toBe('[]');
    expect(JSON.parse(eventsJson([{ id: 1 } as never]))).toEqual({ id: 1 });
  });
});
