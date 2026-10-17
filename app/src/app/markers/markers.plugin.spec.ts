import { TestBed } from '@angular/core/testing';
import { IMT_COMMANDS, IMT_VIEWS, flattenContributions } from '../extensions/tokens';
import { provideTinyBackend, settle, tinyBackend } from '../../testing/helpers';
import { RecordActions } from '../context/record-actions';
import { WorkspaceStore } from '../core/workspace.store';
import { GridSelection } from '../grid/grid-selection';
import { CommandContext } from '../extensions/tokens';
import { provideMarkers, MarkersCommands } from './markers.plugin';
import { MarkersManager } from './markers-manager';

describe('markers plugin', () => {
  const setup = () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(tinyBackend()), provideMarkers()] });
    const commands = flattenContributions(TestBed.inject(IMT_COMMANDS) as never) as { id: string; run(ctx: CommandContext): void }[];
    const ctx = { showView: vi.fn(), notifyError: vi.fn() } as unknown as CommandContext;
    return { commands, ctx, store: TestBed.inject(WorkspaceStore) };
  };

  it('registers a dockable Markers view that loads the manager', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideMarkers()] });
    const views = flattenContributions(TestBed.inject(IMT_VIEWS) as never) as { id: string; title: string; component(): Promise<unknown> }[];
    const v = views.find((x) => x.id === 'markers')!;
    expect(v.title).toBe('Markers');
    expect(await v.component()).toBe(MarkersManager);
  });

  it('manage markers opens the view', () => {
    const { commands, ctx } = setup();
    commands.find((c) => c.id === 'markers.manage')!.run(ctx);
    expect(ctx.showView).toHaveBeenCalledWith('markers');
    expect(TestBed.inject(MarkersCommands).commands.map((c) => c.id)).toEqual(['markers.manage', 'markers.highlight', 'markers.infer']);
  });

  it('highlight and infer need a selected event, then open their dialogs', async () => {
    const { commands, ctx, store } = setup();
    const actions = TestBed.inject(RecordActions);
    const highlight = vi.spyOn(actions, 'highlight').mockImplementation(() => undefined);
    const infer = vi.spyOn(actions, 'infer').mockImplementation(() => undefined);
    commands.find((c) => c.id === 'markers.highlight')!.run(ctx);
    commands.find((c) => c.id === 'markers.infer')!.run(ctx);
    expect(ctx.notifyError).toHaveBeenCalledTimes(2);
    await settle();
    store.select({ id: 1, message: 'm' } as never);
    commands.find((c) => c.id === 'markers.highlight')!.run(ctx);
    commands.find((c) => c.id === 'markers.infer')!.run(ctx);
    expect(highlight).toHaveBeenCalledWith(expect.objectContaining({ id: 1 }), '');
    expect(infer).toHaveBeenCalledWith([expect.objectContaining({ id: 1 })]);
  });

  it('infer uses every line selected in the grid', () => {
    const { commands, ctx, store } = setup();
    const infer = vi.spyOn(TestBed.inject(RecordActions), 'infer').mockImplementation(() => undefined);
    store.select({ id: 2, message: 'b' } as never);
    TestBed.inject(GridSelection).publish([{ id: 1, message: 'a' } as never, { id: 2, message: 'b' } as never]);
    commands.find((c) => c.id === 'markers.infer')!.run(ctx);
    expect(infer).toHaveBeenCalledWith([expect.objectContaining({ id: 1 }), expect.objectContaining({ id: 2 })]);
    expect(ctx.notifyError).not.toHaveBeenCalled();
  });
});
