import { TestBed } from '@angular/core/testing';
import { all, el, provideFakeDock, provideTinyBackendBare, settle, typeInto } from '../../../testing/helpers';
import { CommandRegistry } from '../../extensions/command-registry';
import { KeybindingOverridesStore } from '../../extensions/keybinding-overrides.store';
import { IMT_COMMANDS, ImtCommand } from '../../extensions/tokens';
import { KeyboardSettings } from './keyboard-settings';
import { shortcutRows } from './shortcut-rows';

const cmd = (id: string, title: string, extra: Partial<ImtCommand> = {}): ImtCommand => ({ id, title, run: () => undefined, ...extra });
const COMMANDS = [
  cmd('t.open', 'Open files', { category: 'File', keybinding: 'Mod+O' }),
  cmd('t.save', 'Save', { category: 'File', keybinding: 'Mod+S', when: () => true }),
  cmd('t.plain', 'Plain command', { category: 'View' }),
  cmd('t.nocat', 'Uncategorised'),
];

describe('shortcutRows', () => {
  it('lists every command with its effective and default key, marks changes and scope, sorts and filters', () => {
    const declared = (id: string) => (id === 't.open' ? 'Mod+O' : id === 't.plain' ? 'Mod+P' : undefined);
    const rows = shortcutRows([{ ...COMMANDS[0]!, keybinding: 'Alt+J' }, COMMANDS[1]!, { ...COMMANDS[2]!, keybinding: undefined }, COMMANDS[3]!], declared, '', 'General', false);
    expect(rows.map((r) => r.id)).toEqual(['t.open', 't.save', 't.nocat', 't.plain']);
    expect(rows[0]).toMatchObject({ keys: 'Alt+J', defaultKeys: 'Ctrl+O', changed: true, scoped: false, category: 'File' });
    expect(rows[1]).toMatchObject({ keys: 'Ctrl+S', defaultKeys: '', changed: true, scoped: true });
    expect(rows[2]).toMatchObject({ category: 'General', keys: '', changed: false });
    expect(rows[3]).toMatchObject({ keys: '', defaultKeys: 'Ctrl+P', changed: true });
    expect(shortcutRows(COMMANDS, () => undefined, 'file ctrl+o', 'General', false).map((r) => r.id)).toEqual(['t.open']);
    expect(shortcutRows(COMMANDS, () => undefined, 'zzz', 'General', false)).toEqual([]);
  });
});

describe('KeyboardSettings', () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => localStorage.clear());

  async function setup() {
    TestBed.configureTestingModule({
      providers: [provideTinyBackendBare(), provideFakeDock(), { provide: IMT_COMMANDS, multi: true, useValue: COMMANDS }],
    });
    const fixture = TestBed.createComponent(KeyboardSettings);
    await settle(fixture);
    return { fixture, registry: TestBed.inject(CommandRegistry), store: TestBed.inject(KeybindingOverridesStore) };
  }

  const keyButton = (fixture: Parameters<typeof el>[0], id: string) => el<HTMLButtonElement>(fixture, `tr[data-command="${id}"] .key`);
  const press = (init: KeyboardEventInit) => document.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init }));
  const status = (fixture: Parameters<typeof el>[0]) => el(fixture, '[data-testid=shortcut-status]').textContent?.trim();

  it('shows the commands in a table with current and default keys and notes scoped ones', async () => {
    const { fixture } = await setup();
    const row = el(fixture, 'tr[data-command="t.save"]');
    expect(row.textContent).toContain('Save');
    expect(row.textContent).toContain('(when available)');
    expect(row.textContent).toContain('Ctrl+S');
    typeInto(el<HTMLInputElement>(fixture, 'input'), 'plain');
    await settle(fixture);
    expect(all(fixture, 'tbody tr').length).toBe(1);
    typeInto(el<HTMLInputElement>(fixture, 'input'), 'nope');
    await settle(fixture);
    expect(el(fixture, 'tbody').textContent).toContain('No command matches');
  });

  it('records a new key, suspends the dispatcher meanwhile, and resets', async () => {
    const { fixture, registry, store } = await setup();
    keyButton(fixture, 't.open').click();
    await settle(fixture);
    expect(registry.keysSuspended()).toBe(true);
    expect(keyButton(fixture, 't.open').textContent).toContain('Press keys');
    press({ key: 'Control', ctrlKey: true });
    press({ key: 'j', ctrlKey: true });
    await settle(fixture);
    expect(store.overrides()).toEqual({ 't.open': 'Mod+J' });
    expect(registry.find('t.open')?.keybinding).toBe('Mod+J');
    expect(registry.keysSuspended()).toBe(false);
    expect(status(fixture)).toContain('Open files is now Ctrl+J');
    expect(el(fixture, 'tr[data-command="t.open"]').classList.contains('changed')).toBe(true);

    el<HTMLButtonElement>(fixture, 'tr[data-command="t.open"] .act button').click();
    await settle(fixture);
    expect(store.overrides()).toEqual({});
    expect(status(fixture)).toContain('back to its default');
  });

  it('cancels with Escape, clears with Backspace, explains a key that cannot be bound', async () => {
    const { fixture, store } = await setup();
    keyButton(fixture, 't.open').click();
    await settle(fixture);
    press({ key: 'x' });
    await settle(fixture);
    expect(status(fixture)).toContain('cannot be used on its own');
    press({ key: 'Escape' });
    await settle(fixture);
    expect(store.overrides()).toEqual({});
    expect(keyButton(fixture, 't.open').textContent).toContain('Ctrl+O');

    keyButton(fixture, 't.open').click();
    await settle(fixture);
    press({ key: 'Backspace' });
    await settle(fixture);
    expect(store.overrides()).toEqual({ 't.open': null });
    expect(keyButton(fixture, 't.open').textContent).toContain('None');
    // Clicking the recording button again, or leaving it, stops recording.
    keyButton(fixture, 't.open').click();
    keyButton(fixture, 't.open').click();
    await settle(fixture);
    expect(keyButton(fixture, 't.open').getAttribute('aria-pressed')).toBe('false');
    keyButton(fixture, 't.open').click();
    await settle(fixture);
    keyButton(fixture, 't.open').dispatchEvent(new FocusEvent('blur'));
    await settle(fixture);
    expect(keyButton(fixture, 't.open').getAttribute('aria-pressed')).toBe('false');
  });

  it('lets Tab and Enter through while recording', async () => {
    const { fixture } = await setup();
    keyButton(fixture, 't.open').click();
    await settle(fixture);
    const tab = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true });
    document.dispatchEvent(tab);
    expect(tab.defaultPrevented).toBe(false);
  });

  it('asks before taking a key another command has: Cancel keeps both, Replace moves it', async () => {
    const { fixture, store } = await setup();
    keyButton(fixture, 't.plain').click();
    await settle(fixture);
    press({ key: 's', ctrlKey: true });
    await settle(fixture);
    const dialog = el(fixture, '[data-testid=shortcut-conflict]');
    expect(dialog.textContent).toContain('already used by “Save”');
    expect(store.overrides()).toEqual({});
    expect(document.activeElement).toBe(el(fixture, '[data-testid=shortcut-cancel]'));

    el<HTMLButtonElement>(fixture, '[data-testid=shortcut-cancel]').click();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('[data-testid=shortcut-conflict]')).toBeNull();
    expect(store.overrides()).toEqual({});

    keyButton(fixture, 't.plain').click();
    await settle(fixture);
    press({ key: 's', ctrlKey: true });
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[data-testid=shortcut-replace]').click();
    await settle(fixture);
    expect(store.overrides()).toEqual({ 't.plain': 'Mod+S', 't.save': null });
    expect(status(fixture)).toContain('Plain command is now Ctrl+S');

    el<HTMLButtonElement>(fixture, 'button.btn:not(.ghost)').click();
    await settle(fixture);
    expect(store.overrides()).toEqual({});
  });

  it('cannot take a fixed key, and Escape closes the question', async () => {
    const { fixture, store } = await setup();
    keyButton(fixture, 't.plain').click();
    await settle(fixture);
    press({ key: 'Enter', ctrlKey: true });
    await settle(fixture);
    expect(el(fixture, '[data-testid=shortcut-conflict]').textContent).toContain('built-in key');
    expect(fixture.nativeElement.querySelector('[data-testid=shortcut-replace]')).toBeNull();
    press({ key: 'Escape' });
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('[data-testid=shortcut-conflict]')).toBeNull();
    expect(store.overrides()).toEqual({});
    // A pending question ends when another row starts recording.
    keyButton(fixture, 't.plain').click();
    await settle(fixture);
    press({ key: 's', ctrlKey: true });
    await settle(fixture);
    keyButton(fixture, 't.open').click();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('[data-testid=shortcut-conflict]')).toBeNull();
  });
});
