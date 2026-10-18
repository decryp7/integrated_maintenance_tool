import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, markerKind, provideTinyBackend, settle, text, tinyDataset, typeInto } from '../../testing/helpers';
import { MockBackend } from '../core/mock/mock-backend';
import { ProfileModel } from '../core/profile.model';
import { parseProfile } from '../profile/toml';
import { DIALOG_REF, DialogService } from '../ui/dialog.service';
import { MarkersManager } from './markers-manager';
import { MarkerOverrideDialog } from './marker-override-dialog';
import { isPluginOwned, ownerOfKind, pluginRows, resetStyle, setOverride, styleProblems, toCondition } from './marker-overrides';
import { at } from '../../testing/must';

describe('marker overrides (pure)', () => {
  it('tells plugin kinds from profile and built-in ones', () => {
    expect([isPluginOwned('profile'), isPluginOwned('builtin'), isPluginOwned('net.x')]).toEqual([false, false, true]);
    expect(ownerOfKind('net.x/crash')).toBe('net.x');
    expect(ownerOfKind('crash')).toBe('');
  });

  it('turns engine rule conditions into displayable conditions', () => {
    expect(toCondition({ field: 'EventId', op: 'in', value: ['41', '6008'] })).toEqual({ field: 'EventId', op: 'in', value: '41, 6008' });
    expect(toCondition({ field: 'x', op: 'exists', value: null })).toEqual({ field: 'x', op: 'exists' });
    expect(toCondition({ field: 'n', op: 'gt', value: 5 })).toEqual({ field: 'n', op: 'gt', value: '5' });
  });

  it('writes, merges and removes overrides', () => {
    const m: ProfileModel = {};
    setOverride(m, 'p/a', { enabled: false });
    expect(m.marker_override).toEqual([{ kind: 'p/a', enabled: false }]);
    setOverride(m, 'p/a', { color: '#fff', badge: 'X' });
    setOverride(m, 'p/b', { label: 'B' });
    expect(m.marker_override).toEqual([
      { kind: 'p/a', enabled: false, color: '#fff', badge: 'X' },
      { kind: 'p/b', label: 'B' },
    ]);
    setOverride(m, 'p/a', { enabled: true });
    expect(m.marker_override![0]).toEqual({ kind: 'p/a', color: '#fff', badge: 'X' });
    resetStyle(m, 'p/a');
    setOverride(m, 'p/b', { label: '' });
    expect(m.marker_override).toBeUndefined();
  });

  it('lists active plugin kinds, then the ones switched off', () => {
    const kinds = [
      markerKind({ kind: 'mine' }),
      markerKind({ kind: 'error', owner: 'builtin' }),
      markerKind({ kind: 'p/crash', owner: 'p', label: 'Crash', role: 'crash', when: [{ field: '@level', op: 'eq', value: 'Fatal' }] }),
    ];
    const model: ProfileModel = {
      marker_override: [
        { kind: 'p/crash', color: '#123' },
        { kind: 'p/stop', enabled: false, label: 'Stopped' },
        { kind: 'unnamespaced', enabled: false },
      ],
    };
    const rows = pluginRows(kinds, model);
    expect(rows.map((r) => [r.kind, r.owner, r.enabled, r.label])).toEqual([
      ['p/crash', 'p', true, 'Crash'],
      ['p/stop', 'p', false, 'Stopped'],
    ]);
    expect(at(rows, 0).restyled).toEqual(['color']);
    expect(at(rows, 0).when).toEqual([{ field: '@level', op: 'eq', value: 'Fatal' }]);
    expect(pluginRows(kinds, { marker_override: 'bad' as never })).toHaveLength(1);
  });

  it('checks a restyle early', () => {
    expect(styleProblems({ color: 'red', badge: 'ABCD', shape: 'star' })).toHaveLength(3);
    expect(styleProblems({ color: '#abc', badge: 'AB', shape: 'hexagon' })).toEqual([]);
    expect(styleProblems({})).toEqual([]);
  });
});

describe('MarkersManager plugin kinds', () => {
  async function mount() {
    const backend = new MockBackend({ latency: false, dataset: tinyDataset });
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const fixture = TestBed.createComponent(MarkersManager);
    await settle(fixture, 10);
    return { fixture, backend, dialogs: TestBed.inject(DialogService) };
  }
  const model = async (b: MockBackend) => parseProfile((await b.getProfile()).toml);
  const row = (f: Awaited<ReturnType<typeof mount>>['fixture'], kind: string) => el(f, `.plugin-kinds tr[data-kind="${kind}"]`);

  it('lists plugin kinds with their owner and a read-only rule', async () => {
    const { fixture } = await mount();
    expect(all(fixture, '.plugin-kinds tbody tr').map((r) => r.getAttribute('data-kind'))).toEqual([
      'net.decryptology.windows/app-crash',
      'net.decryptology.windows/service-stop',
    ]);
    const crash = row(fixture, 'net.decryptology.windows/app-crash');
    expect(crash.querySelector('.badge.owner')!.textContent).toBe('net.decryptology.windows');
    expect(crash.textContent).toContain('level is "Fatal"');
    expect(crash.textContent).toContain('read-only');
    expect(crash.textContent).toContain('crash');
    expect(crash.querySelector('.num')!.textContent!.trim()).toBe('1');
    // No edit/delete/reorder for plugin kinds.
    expect(crash.querySelector('[aria-label^="Edit"], [aria-label^="Delete"], [aria-label^="Move"]')).toBeNull();
    expect(text(fixture)).toContain('Plugin markers');
  });

  it('switches a plugin kind off and on through [[marker_override]]', async () => {
    const { fixture, backend } = await mount();
    el<HTMLInputElement>(fixture, '[aria-label="Disable Application crash"]').click();
    await settle(fixture, 10);
    expect((await model(backend)).marker_override).toEqual([{ kind: 'net.decryptology.windows/app-crash', enabled: false }]);
    expect((await backend.workspaceSummary()).markerKinds.some((k) => k.kind === 'net.decryptology.windows/app-crash')).toBe(false);
    const off = row(fixture, 'net.decryptology.windows/app-crash');
    expect(off.classList).toContain('off');
    el<HTMLInputElement>(fixture, '[aria-label="Enable net.decryptology.windows/app-crash"]').click();
    await settle(fixture, 10);
    expect((await model(backend)).marker_override).toBeUndefined();
    expect(row(fixture, 'net.decryptology.windows/app-crash').classList).not.toContain('off');
  });

  it('restyles a plugin kind and resets the style', async () => {
    const { fixture, backend, dialogs } = await mount();
    el<HTMLButtonElement>(fixture, '[aria-label="Restyle Application crash"]').click();
    await settle(fixture);
    const open = dialogs.stack().at(-1)!;
    expect(open.component).toBe(MarkerOverrideDialog);
    open.ref.close({ color: '#00ff00', badge: 'Z', label: undefined });
    await settle(fixture, 10);
    expect((await model(backend)).marker_override).toEqual([{ kind: 'net.decryptology.windows/app-crash', color: '#00ff00', badge: 'Z' }]);
    expect((await backend.workspaceSummary()).markerKinds.find((k) => k.kind.endsWith('/app-crash'))!.badge).toBe('Z');
    el<HTMLButtonElement>(fixture, '[aria-label="Reset style of Application crash"]').click();
    await settle(fixture, 10);
    expect((await model(backend)).marker_override).toBeUndefined();
    // Cancelling the dialog writes nothing.
    el<HTMLButtonElement>(fixture, '[aria-label="Restyle Application crash"]').click();
    await settle(fixture);
    dialogs.stack().at(-1)!.ref.close();
    await settle(fixture, 6);
    expect((await model(backend)).marker_override).toBeUndefined();
  });

  it('shows the engine message when the override is rejected', async () => {
    const { fixture, backend } = await mount();
    vi.spyOn(backend, 'validateProfile').mockResolvedValueOnce({ ok: false, error: 'color "red" is not valid (check [[marker_override]])', line: 3 } as never);
    el<HTMLInputElement>(fixture, '[aria-label="Disable Application crash"]').click();
    await settle(fixture, 10);
    expect(el(fixture, '.err').textContent).toContain('marker_override');
    expect(el(fixture, '.err').textContent).toContain('profile line 3');
  });
});

describe('MarkerOverrideDialog', () => {
  it('previews and returns the style, refusing invalid values', async () => {
    const close = vi.fn();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), { provide: DIALOG_REF, useValue: { close, closed: new Promise(() => undefined) } }] });
    const fixture = TestBed.createComponent(MarkerOverrideDialog);
    const info = markerKind({ kind: 'p/crash', owner: 'p', label: 'Crash', color: '#E5484D', badge: 'AC', shape: 'hexagon' });
    fixture.componentRef.setInput('row', { kind: 'p/crash', owner: 'p', label: 'Crash', enabled: true, info, when: [], restyled: [] });
    fixture.componentRef.setInput('current', { color: '#123456' });
    await settle(fixture);
    expect(text(fixture)).toContain('belongs to the plugin p');
    const input = (name: string) => el<HTMLInputElement>(fixture, `[name="${name}"]`);
    expect(input('color').value).toBe('#123456');
    typeInto(input('color'), 'red');
    typeInto(input('badge'), 'TOOLONG');
    await settle(fixture);
    expect(all(fixture, '.err')).toHaveLength(2);
    expect(buttonByText(fixture, 'Save style').disabled).toBe(true);
    (fixture.componentInstance as unknown as { save(): void }).save();
    expect(close).not.toHaveBeenCalled();
    typeInto(input('color'), '');
    at(all<HTMLButtonElement>(fixture, '.swatch'), 0).click();
    typeInto(input('badge'), 'Z');
    typeInto(input('label'), ' New label ');
    typeInto(el<HTMLSelectElement>(fixture, '[name="shape"]'), 'square', 'change');
    await settle(fixture);
    expect(el(fixture, 'imt-marker-glyph').getAttribute('data-glyph')).toBe('badge');
    buttonByText(fixture, 'Save style').click();
    expect(close).toHaveBeenCalledWith({ label: 'New label', description: undefined, color: '#f2545b', badge: 'Z', shape: 'square' });
    buttonByText(fixture, 'Cancel').click();
    expect(close).toHaveBeenLastCalledWith();
  });
});
