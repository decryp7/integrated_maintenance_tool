import { TestBed } from '@angular/core/testing';
import { all, el, settle, text, typeInto } from '../../testing/helpers';
import { SettingFieldControl } from './setting-field';
import { SettingsFiles } from './settings-files';
import { FieldKind, SettingField, SettingValue } from './settings.types';

const field = (kind: FieldKind, def: SettingValue, extra: Partial<SettingField> = {}): SettingField => ({
  key: 'k',
  label: 'My setting',
  description: 'What it does',
  kind,
  default: def,
  restartRequired: false,
  advanced: false,
  ...extra,
});

async function render(f: SettingField, value: SettingValue | undefined, opts: { error?: string | null; readOnly?: boolean } = {}) {
  const pickPath = vi.fn(async (dir: boolean) => (dir ? '/picked/dir' : null));
  TestBed.configureTestingModule({ providers: [{ provide: SettingsFiles, useValue: { pickPath } }] });
  const fixture = TestBed.createComponent(SettingFieldControl);
  fixture.componentRef.setInput('field', f);
  fixture.componentRef.setInput('value', value);
  fixture.componentRef.setInput('error', opts.error ?? null);
  fixture.componentRef.setInput('readOnly', opts.readOnly ?? false);
  const changed: SettingValue[] = [];
  let resets = 0;
  fixture.componentInstance.changed.subscribe((v) => changed.push(v));
  fixture.componentInstance.resetToDefault.subscribe(() => resets++);
  await settle(fixture);
  return { fixture, changed, resets: () => resets, pickPath };
}

describe('SettingFieldControl', () => {
  it('renders a switch for bool, labelled and described', async () => {
    const { fixture, changed } = await render(field({ type: 'bool' }, true, { restartRequired: true, advanced: true }), false);
    const box = el<HTMLInputElement>(fixture, 'input[type=checkbox]');
    expect(box.getAttribute('role')).toBe('switch');
    expect(box.checked).toBe(false);
    expect(el(fixture, `label[for="${box.id}"]`).textContent).toContain('My setting');
    expect(box.getAttribute('aria-describedby')).toBe(`${box.id}-help`);
    expect(text(fixture)).toContain('restart');
    expect(text(fixture)).toContain('advanced');
    box.click();
    expect(changed).toEqual([true]);
  });

  it('renders an int with unit, limits and a reset button when not default', async () => {
    const { fixture, changed, resets } = await render(field({ type: 'int', min: 1, max: 100, step: 1, unit: 'MB' }, 10), 20);
    const input = el<HTMLInputElement>(fixture, 'input[type=number]');
    expect([input.min, input.max, input.step, input.value]).toEqual(['1', '100', '1', '20']);
    expect(text(fixture)).toContain('MB');
    expect(text(fixture)).toContain('1–100 MB');
    typeInto(input, '30');
    // A browser empties a number input holding text; the engine then rejects the empty value.
    typeInto(input, 'abc');
    expect(changed).toEqual([30, '']);
    const reset = el<HTMLButtonElement>(fixture, 'button.reset');
    expect(reset.getAttribute('aria-label')).toBe('Reset My setting to default');
    expect(reset.title).toBe('Default: 10 MB');
    reset.click();
    expect(resets()).toBe(1);
  });

  it('hides reset at the default', async () => {
    const { fixture } = await render(field({ type: 'float', min: 0, max: 1, step: 0.1, unit: null }, 0.5), 0.5);
    expect(all(fixture, 'button.reset')).toHaveLength(0);
    expect(el<HTMLInputElement>(fixture, 'input').step).toBe('0.1');
  });

  it('edits durations in seconds and sizes in MB, sending ms and bytes', async () => {
    const d = await render(field({ type: 'duration', minMs: 5000, maxMs: 3_600_000 }, 120_000), 120_000);
    const input = el<HTMLInputElement>(d.fixture, 'input');
    expect([input.value, input.min, input.max]).toEqual(['120', '5', '3600']);
    expect(text(d.fixture)).toContain('5 s – 1 h');
    typeInto(input, '300');
    expect(d.changed).toEqual([300_000]);
    TestBed.resetTestingModule();
    const s = await render(field({ type: 'size', minBytes: 1 << 20, maxBytes: 1 << 30 }, 64 << 20), 64 << 20);
    const si = el<HTMLInputElement>(s.fixture, 'input');
    expect(si.value).toBe('64');
    typeInto(si, '128');
    expect(s.changed).toEqual([128 << 20]);
    expect(text(s.fixture)).toContain('MB');
  });

  it('renders text, enum and string lists', async () => {
    const t = await render(field({ type: 'text', maxLen: 5, pattern: null }, ''), 'abc');
    typeInto(el<HTMLInputElement>(t.fixture, 'input[type=text]'), 'abcd');
    expect(t.changed).toEqual(['abcd']);
    expect(text(t.fixture)).toContain('up to 5 characters');
    TestBed.resetTestingModule();
    const e = await render(field({ type: 'enum', options: [{ value: 'a', label: 'Alpha' }, { value: 'b', label: 'Beta' }] }, 'a'), 'b');
    const select = el<HTMLSelectElement>(e.fixture, 'select');
    expect(select.value).toBe('b');
    typeInto(select, 'a', 'change');
    expect(e.changed).toEqual(['a']);
    TestBed.resetTestingModule();
    const l = await render(field({ type: 'stringList', maxItems: 5 }, []), ['x', 'y']);
    const area = el<HTMLTextAreaElement>(l.fixture, 'textarea');
    expect(area.value).toBe('x\ny');
    typeInto(area, 'a\n\nb');
    expect(l.changed).toEqual([['a', 'b']]);
  });

  it('renders paths with a Browse button', async () => {
    const p = await render(field({ type: 'path', dir: true }, ''), '');
    const input = el<HTMLInputElement>(p.fixture, 'input[type=text]');
    expect(input.placeholder).toContain('Folder');
    typeInto(input, '/var/log');
    el<HTMLButtonElement>(p.fixture, 'button[aria-label="Browse for My setting"]').click();
    await settle(p.fixture);
    expect(p.changed).toEqual(['/var/log', '/picked/dir']);
    expect(p.pickPath).toHaveBeenCalledWith(true, 'My setting');
    TestBed.resetTestingModule();
    const f = await render(field({ type: 'path', dir: false }, ''), '');
    el<HTMLButtonElement>(f.fixture, 'button[aria-label="Browse for My setting"]').click();
    await settle(f.fixture);
    expect(f.changed).toEqual([]);
    expect(el<HTMLInputElement>(f.fixture, 'input').placeholder).toContain('File');
  });

  it('shows the engine error, else a pre-check, as an alert tied to the control', async () => {
    const { fixture } = await render(field({ type: 'int', min: 1, max: 10, step: 1, unit: null }, 1), 50, { error: 'Must be between 1 and 10.' });
    const input = el<HTMLInputElement>(fixture, 'input');
    expect(input.getAttribute('aria-invalid')).toBe('true');
    expect(input.getAttribute('aria-describedby')).toBe(`${input.id}-help ${input.id}-error`);
    expect(el(fixture, '[role=alert]').textContent).toContain('Must be between 1 and 10.');
    fixture.componentRef.setInput('error', null);
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('Between 1–10.');
  });

  it('is read-only without reset or pre-check', async () => {
    const { fixture } = await render(field({ type: 'int', min: 1, max: 10, step: 1, unit: null }, 1, { description: '' }), 50, { readOnly: true });
    const input = el<HTMLInputElement>(fixture, 'input');
    expect(input.disabled).toBe(true);
    expect(input.getAttribute('aria-describedby')).toBeNull();
    expect(all(fixture, 'button.reset')).toHaveLength(0);
    expect(all(fixture, '[role=alert]')).toHaveLength(0);
  });

  it('copes with a missing value', async () => {
    const t = await render(field({ type: 'text', maxLen: 5, pattern: null }, ''), undefined);
    expect(el<HTMLInputElement>(t.fixture, 'input').value).toBe('');
    TestBed.resetTestingModule();
    const n = await render(field({ type: 'int', min: 0, max: 5, step: 1, unit: null }, 0), undefined);
    expect(el<HTMLInputElement>(n.fixture, 'input').value).toBe('');
    TestBed.resetTestingModule();
    const l = await render(field({ type: 'stringList', maxItems: 5 }, []), undefined);
    expect(el<HTMLTextAreaElement>(l.fixture, 'textarea').value).toBe('');
    TestBed.resetTestingModule();
    const w = await render(field({ type: 'text', maxLen: 5, pattern: null }, ''), 5);
    expect(el<HTMLInputElement>(w.fixture, 'input').value).toBe('5');
  });
});
