import { TestBed } from '@angular/core/testing';
import { all, el, provideFakeDock, provideTinyBackendBare, settle, text, typeInto } from '../../testing/helpers';
import { CommandRegistry } from '../extensions/command-registry';
import { KeybindingOverridesStore } from '../extensions/keybinding-overrides.store';
import { DEBRIEF_COMMANDS, DebriefCommand } from '../extensions/tokens';
import { DialogService } from '../ui/dialog.service';
import { DockService } from '../dock/dock.service';
import { CheatSheet, PRINT_CLASS } from './cheat-sheet';
import { HelpCommands, provideHelp, SHORTCUTS_KEY } from './help.commands';
import { FIXED_KEYS, shortcutGroups } from './shortcuts';
import { SettingsDialogService } from '../settings/settings-dialog';
import { at } from '../../testing/must';

const cmd = (id: string, title: string, extra: Partial<DebriefCommand> = {}): DebriefCommand => ({ id, title, run: () => undefined, ...extra });

describe('shortcutGroups', () => {
  const commands = [
    cmd('a', 'Open files…', { category: 'File', keybinding: 'Mod+O' }),
    cmd('a', 'Duplicate id', { category: 'File', keybinding: 'Mod+X' }),
    cmd('b', 'Toggle sidebar', { category: 'View', keybinding: 'Mod+B' }),
    cmd('c', 'No key', { category: 'View' }),
    cmd('d', 'Uncategorised', { keybinding: 'F2' }),
  ];

  it('groups bound commands by category with formatted keys, plus the fixed keys, sorted', () => {
    const groups = shortcutGroups(commands, [{ group: 'Timeline', binding: 'Alt+Left', description: 'Back' }, { group: 'General', text: '/', description: 'Search' }], '', false);
    expect(groups.map((g) => g.title)).toEqual(['File', 'General', 'Timeline', 'View']);
    expect(at(groups, 0).items).toEqual([{ keys: 'Ctrl+O', description: 'Open files…' }]);
    expect(at(groups, 1).items.map((i) => i.keys)).toEqual(['/', 'F2']);
    expect(at(at(groups, 2).items, 0).keys).toBe('Alt+←');
    expect(at(at(shortcutGroups(commands, [], '', true), 0).items, 0).keys).toBe('⌘O');
  });

  it('filters by every word over group, description and keys, dropping empty groups', () => {
    expect(shortcutGroups(commands, FIXED_KEYS, 'toggle side').map((g) => g.items.map((i) => i.description))).toEqual([['Toggle sidebar']]);
    expect(shortcutGroups(commands, FIXED_KEYS, 'timeline').flatMap((g) => g.items).length).toBeGreaterThan(1);
    expect(shortcutGroups(commands, FIXED_KEYS, 'nothing-like-this')).toEqual([]);
  });
});

describe('cheat sheet and help commands', () => {
  function setup() {
    TestBed.configureTestingModule({
      providers: [
        provideTinyBackendBare(),
        provideFakeDock(),
        provideHelp(),
        { provide: DEBRIEF_COMMANDS, multi: true, useValue: [cmd('t.open', 'Open files…', { category: 'File', keybinding: 'Mod+O' })] },
      ],
    });
    return TestBed.inject(CommandRegistry);
  }

  it('opens the sheet with F1 (once), and Manage parsers… opens the Parsers view', async () => {
    const registry = setup();
    expect(SHORTCUTS_KEY).toBe('F1');
    expect(registry.runForKey(new KeyboardEvent('keydown', { key: 'F1' }))).toBe(true);
    const dialogs = TestBed.inject(DialogService);
    await vi.waitFor(() => expect(dialogs.stack().length).toBe(1));
    expect(at(dialogs.stack(), 0).options.title).toBe('Keyboard shortcuts');
    await registry.run('help.shortcuts');
    expect(dialogs.stack().length).toBe(1);
    const open = vi.spyOn(TestBed.inject(SettingsDialogService), 'open').mockReturnValue(undefined);
    // Manage parsers… opens the Parsers view, not the Settings dialog (I3).
    expect(await registry.run('parsers.manage')).toBe(true);
    expect(TestBed.inject(DockService)).toBeDefined();
    expect((TestBed.inject(DockService) as unknown as { calls: string[] }).calls).toContain('show:parsers');
    expect(open).not.toHaveBeenCalledWith('@parsers');
    expect(await registry.run('help.editShortcuts')).toBe(true);
    expect(open).toHaveBeenCalledWith('@keyboard');
    expect(TestBed.inject(HelpCommands).commands.map((c) => c.id)).toEqual(['help.shortcuts', 'help.whatsNew', 'help.editShortcuts', 'parsers.manage']);
  });

  it('lists registered bindings and fixed keys, searches, clears and prints only the sheet', async () => {
    setup();
    const fixture = TestBed.createComponent(CheatSheet);
    await settle(fixture);
    expect(text(fixture)).toContain('Open files…');
    expect(text(fixture)).toContain('Keyboard shortcuts');
    expect(text(fixture)).toContain('Focus the search box');
    typeInto(el<HTMLInputElement>(fixture, 'input'), 'open files');
    await settle(fixture);
    expect(all(fixture, '.cs-row').length).toBe(1);
    expect(el(fixture, '.cs-count').textContent).toContain('1 shortcut');
    typeInto(el<HTMLInputElement>(fixture, 'input'), 'zzzz');
    await settle(fixture);
    expect(text(fixture)).toContain('No shortcut matches');
    (fixture.nativeElement.querySelector('.cs-none button') as HTMLButtonElement).click();
    await settle(fixture);
    expect(all(fixture, '.cs-row').length).toBeGreaterThan(5);

    vi.useFakeTimers();
    let printing = false;
    const print = vi.spyOn(window, 'print').mockImplementation(() => void (printing = document.body.classList.contains(PRINT_CLASS)));
    (fixture.nativeElement.querySelector('.cs-bar button') as HTMLButtonElement).click();
    expect(print).toHaveBeenCalled();
    expect(printing).toBe(true);
    window.dispatchEvent(new Event('afterprint'));
    expect(document.body.classList.contains(PRINT_CLASS)).toBe(false);
    vi.runAllTimers();
    vi.useRealTimers();
  });
});

describe('cheat sheet with user key bindings', () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => localStorage.clear());

  it('shows the effective binding, omits a cleared one and links to the editor', async () => {
    TestBed.configureTestingModule({
      providers: [
        provideTinyBackendBare(),
        provideFakeDock(),
        provideHelp(),
        {
          provide: DEBRIEF_COMMANDS,
          multi: true,
          useValue: [cmd('t.open', 'Open files…', { category: 'File', keybinding: 'Mod+O' }), cmd('t.save', 'Save things', { category: 'File', keybinding: 'Mod+S' })],
        },
      ],
    });
    TestBed.inject(KeybindingOverridesStore).set({ 't.open': 'Alt+J', 't.save': null });
    const registry = TestBed.inject(CommandRegistry);
    expect(registry.find('t.open')?.keybinding).toBe('Alt+J');
    expect(registry.declaredKeybinding('t.open')).toBe('Mod+O');
    expect(registry.find('t.save')?.keybinding).toBeUndefined();
    // The dispatcher honours the override and the cleared key, and pauses while the editor records.
    expect(registry.runForKey(new KeyboardEvent('keydown', { key: 'o', ctrlKey: true }))).toBe(false);
    registry.keysSuspended.set(true);
    expect(registry.runForKey(new KeyboardEvent('keydown', { key: 'F1' }))).toBe(false);
    registry.keysSuspended.set(false);
    expect(registry.runForKey(new KeyboardEvent('keydown', { key: 'j', altKey: true }))).toBe(true);

    const fixture = TestBed.createComponent(CheatSheet);
    await settle(fixture);
    const rows = all(fixture, '.cs-row').map((r) => r.textContent);
    expect(rows.some((r) => r?.includes('Alt+J') && r.includes('Open files…'))).toBe(true);
    expect(rows.some((r) => r?.includes('Save things'))).toBe(false);
    const open = vi.spyOn(TestBed.inject(SettingsDialogService), 'open').mockReturnValue(undefined);
    (fixture.nativeElement.querySelector('[data-testid=cheat-sheet-edit]') as HTMLButtonElement).click();
    expect(open).toHaveBeenCalledWith('@keyboard');
  });
});
