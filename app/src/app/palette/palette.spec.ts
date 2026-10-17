import { TestBed } from '@angular/core/testing';
import { all, el, FakeDock, provideFakeDock, provideTinyBackendBare, settle, typeInto } from '../../testing/helpers';
import { COMMAND_CONTEXT } from '../extensions';
import { IMT_COMMANDS, ImtCommand } from '../extensions/tokens';
import { CommandPalette } from './command-palette';
import { PaletteService } from './palette.service';

describe('CommandPalette', () => {
  let ran: string[];

  async function render(extra: ImtCommand[] = []) {
    ran = [];
    const mk = (id: string, title: string, extraProps: Partial<ImtCommand> = {}): ImtCommand => ({ id, title, run: () => void ran.push(id), ...extraProps });
    TestBed.configureTestingModule({
      providers: [
        provideTinyBackendBare(),
        provideFakeDock(new FakeDock()),
        { provide: IMT_COMMANDS, multi: true, useValue: [mk('t.alpha', 'Alpha thing', { category: 'Test', keybinding: 'Mod+Shift+A' }), mk('t.beta', 'Beta thing'), mk('t.gamma', 'Gamma other'), ...extra] },
      ],
    });
    const fixture = TestBed.createComponent(CommandPalette);
    const palette = TestBed.inject(PaletteService);
    await settle(fixture);
    return { fixture, palette };
  }

  const options = (f: { nativeElement: HTMLElement }) => [...f.nativeElement.querySelectorAll('[role=option] .title')].map((t) => t.textContent);
  const press = (f: { nativeElement: HTMLElement }, key: string) => (f.nativeElement.querySelector('input') as HTMLInputElement).dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));

  it('is closed by default and opens as an accessible combobox listing commands with key hints', async () => {
    const { fixture, palette } = await render();
    expect(fixture.nativeElement.querySelector('[role=dialog]')).toBeNull();
    palette.open();
    await settle(fixture);
    const input = el<HTMLInputElement>(fixture, 'input');
    expect(input.getAttribute('role')).toBe('combobox');
    expect(document.activeElement).toBe(input);
    expect(options(fixture)).toContain('Alpha thing');
    expect(el(fixture, '[role=option] kbd').textContent).toMatch(/A/);
    expect(el(fixture, '[role=listbox]').id).toBe(input.getAttribute('aria-controls'));
    expect(input.getAttribute('aria-activedescendant')).toBe(el(fixture, '[role=option]').id);
  });

  it('filters fuzzily as you type and shows an empty message', async () => {
    const { fixture, palette } = await render();
    palette.open();
    await settle(fixture);
    typeInto(el<HTMLInputElement>(fixture, 'input'), 'thing');
    await settle(fixture);
    expect(options(fixture).sort()).toEqual(['Alpha thing', 'Beta thing']);
    typeInto(el<HTMLInputElement>(fixture, 'input'), 'zzzz');
    await settle(fixture);
    expect(options(fixture)).toEqual([]);
    expect(fixture.nativeElement.textContent).toContain('No matching commands');
    expect(el<HTMLInputElement>(fixture, 'input').hasAttribute('aria-activedescendant')).toBe(false);
    press(fixture, 'ArrowDown');
    press(fixture, 'ArrowUp');
    press(fixture, 'Enter');
    expect(ran).toEqual([]);
  });

  it('navigates with arrows (wrapping) and runs the highlighted command with Enter', async () => {
    const { fixture, palette } = await render();
    palette.open();
    await settle(fixture);
    typeInto(el<HTMLInputElement>(fixture, 'input'), 'thing');
    await settle(fixture);
    const [first, second] = options(fixture);
    expect(el(fixture, '[role=option].active .title').textContent).toBe(first);
    press(fixture, 'ArrowDown');
    await settle(fixture);
    expect(el(fixture, '[role=option].active .title').textContent).toBe(second);
    press(fixture, 'ArrowDown');
    await settle(fixture);
    expect(el(fixture, '[role=option].active .title').textContent).toBe(first);
    press(fixture, 'ArrowUp');
    await settle(fixture);
    expect(el(fixture, '[role=option].active .title').textContent).toBe(second);
    press(fixture, 'Tab');
    press(fixture, 'a');
    press(fixture, 'Enter');
    await settle(fixture);
    expect(ran).toEqual([second === 'Beta thing' ? 't.beta' : 't.alpha']);
    expect(palette.isOpen()).toBe(false);
  });

  it('runs a command on click, highlights on hover and closes on Escape or backdrop', async () => {
    const { fixture, palette } = await render();
    palette.open();
    await settle(fixture);
    const opts = all(fixture, '[role=option]');
    opts[2].dispatchEvent(new MouseEvent('mousemove', { bubbles: true }));
    await settle(fixture);
    expect(opts[2].classList.contains('active')).toBe(true);
    opts[1].click();
    await settle(fixture);
    expect(ran).toEqual(['t.beta']);
    expect(palette.isOpen()).toBe(false);

    palette.open();
    await settle(fixture);
    press(fixture, 'Escape');
    await settle(fixture);
    expect(palette.isOpen()).toBe(false);

    palette.open();
    await settle(fixture);
    el(fixture, '.backdrop').dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    await settle(fixture);
    expect(palette.isOpen()).toBe(false);
  });

  it('returns focus to where it was when dismissed, and resets the query on reopen', async () => {
    const { fixture, palette } = await render();
    const opener = document.createElement('button');
    document.body.appendChild(opener);
    opener.focus();
    palette.open();
    await settle(fixture);
    typeInto(el<HTMLInputElement>(fixture, 'input'), 'beta');
    press(fixture, 'Escape');
    await settle(fixture);
    expect(document.activeElement).toBe(opener);
    palette.open();
    await settle(fixture);
    expect(el<HTMLInputElement>(fixture, 'input').value).toBe('');
    opener.remove();
  });

  it('lists only available commands and surfaces command failures', async () => {
    const { fixture, palette } = await render([
      { id: 't.hidden', title: 'Hidden one', when: () => false, run: () => undefined },
      { id: 't.fail', title: 'Failing one', run: () => { throw new Error('boom'); } },
    ]);
    palette.open();
    await settle(fixture);
    expect(options(fixture)).not.toContain('Hidden one');
    typeInto(el<HTMLInputElement>(fixture, 'input'), 'failing');
    await settle(fixture);
    press(fixture, 'Enter');
    await settle(fixture);
    expect(TestBed.inject(COMMAND_CONTEXT).summary).toBeTruthy();
  });
});
