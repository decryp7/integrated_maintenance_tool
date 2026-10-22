import { TestBed } from '@angular/core/testing';
import { all, provideTinyBackend, settle, tinyBackend } from '../../testing/helpers';
import { at } from '../../testing/must';
import { WorkspaceStore } from '../core/workspace.store';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { MockParsersBackend } from '../settings/parsers/mock-parsers-backend';
import { ParsersBackend } from '../settings/parsers/parsers-backend';
import { ParsersStore } from '../settings/parsers/parsers.store';
import { ContextMenuService } from '../ui/context-menu.service';
import { Sidebar } from './sidebar';

describe('Explorer ▸ Parse with… (I3)', () => {
  it('opens the Parsers view with that source as the one being chosen for, instead of a dialog of its own', async () => {
    const imt = tinyBackend();
    const showView = vi.fn();
    TestBed.configureTestingModule({
      providers: [provideTinyBackend(imt), { provide: ParsersBackend, useValue: new MockParsersBackend(imt) }, { provide: COMMAND_CONTEXT, useValue: { showView } }],
    });
    await TestBed.inject(WorkspaceStore).init();
    const fixture = TestBed.createComponent(Sidebar);
    await settle(fixture);
    const menu = TestBed.inject(ContextMenuService);
    at(all(fixture, '[data-testid=source-actions]'), 1).click();
    menu.menu()!.items.find((i) => i.label === 'Parse with…')!.run();
    await vi.waitFor(() => expect(showView).toHaveBeenCalledWith('parsers'));
    const source = at(TestBed.inject(WorkspaceStore).summary()!.sources, 1);
    expect(TestBed.inject(ParsersStore).readTarget()).toEqual({ id: source.id, name: source.name });
  });
});
