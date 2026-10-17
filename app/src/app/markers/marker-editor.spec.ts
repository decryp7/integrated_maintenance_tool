import { Provider } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend, typeInto } from '../../testing/helpers';
import { MockBackend } from '../core/mock/mock-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { parseProfile } from '../profile/toml';
import { IconFilePicker } from './icon-import';
import { MarkerEditor } from './marker-editor';
import { MarkerRuleX } from './markers.types';

const slow = [{ field: '@message', op: 'matches' as const, value: 'Slow request' }];

async function mount(inputs: Record<string, unknown> = {}, backend: MockBackend = tinyBackend(), extra: Provider[] = []) {
  TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), ...extra] });
  const fixture = TestBed.createComponent(MarkerEditor);
  for (const [k, v] of Object.entries({ initial: slow, ...inputs })) fixture.componentRef.setInput(k, v);
  const saved = vi.fn();
  const cancelled = vi.fn();
  fixture.componentInstance.saved.subscribe(saved);
  fixture.componentInstance.cancelled.subscribe(cancelled);
  await settle(fixture, 6);
  await new Promise((r) => setTimeout(r, 300));
  await settle(fixture);
  return { fixture, backend, saved, cancelled };
}

const rules = async (backend: MockBackend) => (parseProfile((await backend.getProfile()).toml).marker ?? []) as MarkerRuleX[];
const tab = (f: Parameters<typeof el>[0], name: string) => all<HTMLButtonElement>(f, '[role=tab]').find((t) => t.textContent!.trim() === name)!;

describe('MarkerEditor', () => {
  it('previews the matches, suggests the next free badge and saves a regex rule', async () => {
    const { fixture, backend, saved } = await mount({ suggestedLabel: 'Slow requests' });
    expect(text(fixture)).toMatch(/This marks\s*1\s*events/);
    expect(all(fixture, '.samples li')).toHaveLength(1);
    expect(el<HTMLInputElement>(fixture, 'input[maxlength="3"]').value).toBe('1');
    expect(el<HTMLInputElement>(fixture, 'input.mono').value).toBe('slow-requests');
    typeInto(el<HTMLInputElement>(fixture, 'input[placeholder="What does this marker mean?"]'), 'Requests over the SLA');
    all<HTMLButtonElement>(fixture, '.shapes .pick')[3].click();
    all<HTMLButtonElement>(fixture, '.swatch')[3].click();
    el<HTMLInputElement>(fixture, 'input[type=checkbox]').click();
    await settle(fixture);
    buttonByText(fixture, 'Add marker').click();
    await settle(fixture, 8);
    expect(saved).toHaveBeenCalled();
    const rule = (await rules(backend)).find((r) => r.kind === 'slow-requests')!;
    expect(rule).toMatchObject({ label: 'Slow requests', description: 'Requests over the SLA', badge: '1', shape: 'hexagon', color: '#3987e5', lifecycle: true, when: slow });
    expect(TestBed.inject(WorkspaceStore).notice()).toContain('Added the "Slow requests" marker');
    expect(TestBed.inject(WorkspaceStore).summary()!.markerKinds.find((k) => k.kind === 'slow-requests')).toMatchObject({ badge: '1', description: 'Requests over the SLA' });
  });

  it('switches to a built-in icon, searches, and omits badge and shape', async () => {
    const { fixture, backend } = await mount();
    tab(fixture, 'Icon').click();
    await settle(fixture);
    typeInto(el<HTMLInputElement>(fixture, 'input[type=search]'), 'sql');
    await settle(fixture);
    expect(all(fixture, '[aria-label="Built-in icons"] .pick')).toHaveLength(1);
    typeInto(el<HTMLInputElement>(fixture, 'input[type=search]'), 'zzzz');
    await settle(fixture);
    expect(text(fixture)).toContain('No icon matches');
    typeInto(el<HTMLInputElement>(fixture, 'input[type=search]'), '');
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[aria-label="rocket"]').click();
    await settle(fixture);
    buttonByText(fixture, 'Add marker').click();
    await settle(fixture, 8);
    const rule = (await rules(backend))[0];
    expect(rule.icon).toBe('rocket');
    expect(rule).not.toHaveProperty('badge');
    expect(rule).not.toHaveProperty('shape');
  });

  it('moves between tabs with the arrow keys', async () => {
    const { fixture } = await mount();
    const t = tab(fixture, 'Badge');
    t.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }));
    await settle(fixture);
    expect(tab(fixture, 'Icon').getAttribute('aria-selected')).toBe('true');
    tab(fixture, 'Icon').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true, cancelable: true }));
    tab(fixture, 'Badge').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true, cancelable: true }));
    await settle(fixture);
    expect(tab(fixture, 'Custom').getAttribute('aria-selected')).toBe('true');
    tab(fixture, 'Custom').dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true, cancelable: true }));
    expect(tab(fixture, 'Custom').getAttribute('aria-selected')).toBe('true');
  });

  it('imports a custom svg, previews it and saves it as an [[icon]] sanitised by the engine', async () => {
    const svg = '<svg viewBox="0 0 24 24"><script>alert(1)</script><path d="M2 2L20 20" stroke="red"/></svg>';
    const picker = { pick: async () => new File([svg], 'Acme Logo.svg') };
    const { fixture, backend } = await mount({}, tinyBackend(), [{ provide: IconFilePicker, useValue: picker }]);
    tab(fixture, 'Custom').click();
    await settle(fixture);
    buttonByText(fixture, /Import SVG or PNG/).click();
    await settle(fixture, 6);
    expect(text(fixture)).toContain('Acme-Logo');
    expect(el(fixture, '.current imt-marker-glyph').innerHTML).not.toContain('script');
    buttonByText(fixture, 'Add marker').click();
    await settle(fixture, 8);
    const model = parseProfile((await backend.getProfile()).toml);
    expect(model['icon']).toEqual([{ name: 'Acme-Logo', svg }]);
    expect((model.marker as MarkerRuleX[])[0].icon).toBe('Acme-Logo');
    const stored = TestBed.inject(WorkspaceStore).summary() as unknown as { icons: { name: string; svg: string }[] };
    expect(stored.icons[0].svg).not.toContain('script');
    expect(stored.icons[0].svg).toContain('<path');
  });

  it('shows import errors and lets an existing custom icon be reused', async () => {
    const files = [new File(['x'.repeat(20000)], 'big.svg'), new File(['<svg viewBox="0 0 1 1"><path d="M0 0"/></svg>'], 'a.svg')];
    const picker = { pick: async () => files.shift() ?? null };
    const { fixture, backend } = await mount({}, tinyBackend(), [{ provide: IconFilePicker, useValue: picker }]);
    tab(fixture, 'Custom').click();
    await settle(fixture);
    buttonByText(fixture, /Import SVG or PNG/).click();
    await settle(fixture, 4);
    expect(el(fixture, '[role=alert]').textContent).toContain('16 KB');
    buttonByText(fixture, /Import SVG or PNG/).click();
    await settle(fixture, 6);
    buttonByText(fixture, 'Add marker').click();
    await settle(fixture, 8);
    expect(await rules(backend)).toBeTruthy();
    // A second editor sees the saved icon in the profile and can pick it without importing.
    TestBed.resetTestingModule();
    const again = await mount({}, backend);
    tab(again.fixture, 'Custom').click();
    await settle(again.fixture);
    el<HTMLButtonElement>(again.fixture, '[aria-label="Custom icons in this profile"] .pick').click();
    await settle(again.fixture);
    expect(el<HTMLButtonElement>(again.fixture, '[aria-label="Custom icons in this profile"] .pick').getAttribute('aria-pressed')).toBe('true');
  });

  it('shows the engine failure with its line and keeps the form open', async () => {
    const { fixture, backend, saved } = await mount();
    vi.spyOn(backend, 'validateProfile').mockResolvedValueOnce({ ok: false, error: 'marker "x": unknown icon', line: 1 });
    buttonByText(fixture, 'Add marker').click();
    await settle(fixture, 6);
    expect(el(fixture, '[role=alert]').textContent).toMatch(/unknown icon.*profile line 1/);
    expect(saved).not.toHaveBeenCalled();
    expect(buttonByText(fixture, 'Add marker').disabled).toBe(false);
  });

  it('blocks saving without conditions and reports early problems', async () => {
    const { fixture, cancelled } = await mount({ initial: [] });
    expect(buttonByText(fixture, 'Add marker').disabled).toBe(true);
    buttonByText(fixture, 'Cancel').click();
    expect(cancelled).toHaveBeenCalled();
  });

  it('reports early problems such as a bad badge', async () => {
    const { fixture, saved } = await mount();
    typeInto(el<HTMLInputElement>(fixture, 'input[maxlength="3"]'), 'ABCD');
    buttonByText(fixture, 'Add marker').click();
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('1 to 3 characters');
    expect(saved).not.toHaveBeenCalled();
  });

  it('edits an existing rule in place and keeps its position and unknown keys', async () => {
    const backend = tinyBackend();
    const existing = (await rules(backend)).find((r) => r.kind === 'error')!;
    const { fixture } = await mount({ rule: { ...existing, extra_key: 'keep' }, submitLabel: 'Save marker' }, backend);
    expect(el<HTMLInputElement>(fixture, 'input.mono').value).toBe('error');
    all<HTMLButtonElement>(fixture, '.shapes .pick')[1].click();
    typeInto(el<HTMLInputElement>(fixture, 'input[maxlength="3"]'), 'E');
    await settle(fixture);
    buttonByText(fixture, 'Save marker').click();
    await settle(fixture, 8);
    const list = await rules(backend);
    const i = list.findIndex((r) => r.kind === 'error');
    expect(i).toBe(4);
    expect(list[i]).toMatchObject({ badge: 'E', shape: 'square', extra_key: 'keep' });
    expect(TestBed.inject(WorkspaceStore).notice()).toContain('Updated the');
  });

  it('opens an existing custom-icon rule on the Custom tab and an icon rule on Icon', async () => {
    const a = await mount({ rule: { kind: 'k', icon: 'bug', when: slow } });
    expect(tab(a.fixture, 'Icon').getAttribute('aria-selected')).toBe('true');
    TestBed.resetTestingModule();
    const b = await mount({ rule: { kind: 'k', icon: 'acme', when: slow } });
    expect(tab(b.fixture, 'Custom').getAttribute('aria-selected')).toBe('true');
  });

  it('previews a query-language expression and converts simple ones to conditions', async () => {
    const { fixture } = await mount();
    buttonByText(fixture, /Advanced: query language/).click();
    await settle(fixture);
    const ta = el<HTMLTextAreaElement>(fixture, 'textarea');
    typeInto(ta, 'level = "Error" | stats count()');
    await settle(fixture);
    expect(buttonByText(fixture, 'Convert to conditions').disabled).toBe(true);
    expect(buttonByText(fixture, 'Add marker').disabled).toBe(true);
    expect(text(fixture)).toContain('too complex');
    typeInto(ta, 'level = "Error" and message contains "capture"');
    await settle(fixture);
    expect(buttonByText(fixture, 'Add marker').disabled).toBe(false);
    buttonByText(fixture, 'Convert to conditions').click();
    await settle(fixture);
    expect(all(fixture, 'imt-conditions-editor .cond')).toHaveLength(2);
    buttonByText(fixture, /Advanced: query language/).click();
    await settle(fixture);
    buttonByText(fixture, 'Back to the condition builder').click();
    await settle(fixture);
    expect(all(fixture, 'imt-conditions-editor .cond')).toHaveLength(2);
  });

  it('saves a simple query-language expression as conditions', async () => {
    const { fixture, backend } = await mount({ suggestedLabel: 'Q' });
    buttonByText(fixture, /Advanced: query language/).click();
    await settle(fixture);
    typeInto(el<HTMLTextAreaElement>(fixture, 'textarea'), 'level = "Error"');
    await settle(fixture);
    buttonByText(fixture, 'Add marker').click();
    await settle(fixture, 8);
    expect((await rules(backend))[0].when).toEqual([{ field: '@level', op: 'eq', value: 'Error' }]);
  });

  it('shows a preview error when the count fails and offers a regex explanation', async () => {
    const backend = tinyBackend();
    vi.spyOn(backend, 'queryPage').mockRejectedValue(new Error('engine down'));
    const { fixture } = await mount({}, backend);
    expect(text(fixture)).toContain('Could not count: engine down');
    buttonByText(fixture, 'Explain').click();
    await settle(fixture, 4);
    expect(text(fixture)).toContain('Literal "Slow request"');
  });
});
