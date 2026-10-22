import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideFakeDock, provideTinyBackend, settle, T0, text, tinyDataset, typeInto } from '../../testing/helpers';
import { EMPTY_QUERY, WorkspaceStore } from '../core/workspace.store';
import { MockBackend } from '../core/mock/mock-backend';
import { mockUnsupported } from '../core/mock/mock-unsupported';
import { MockDataset } from '../core/mock/mock-data';
import { Platform } from '../core/platform';
import { RecordView } from '../core/api.types';
import { SkewBackend } from '../core/skew-backend';
import { CommandRegistry } from '../extensions/command-registry';
import { Inspector } from '../inspector/inspector';
import { Sidebar } from '../shell/sidebar';
import { ContextMenuService } from '../ui/context-menu.service';
import { DIALOG_REF, DialogService } from '../ui/dialog.service';
import { ClockDialog } from './clock-dialog';
import { ClockDialogService, ClockSection } from './clock-section';
import { ClockSkewStore } from './clock-skew.store';
import { confidenceLabel, formatConfidence, formatOffset, parseOffset } from './skew-format';
import { CLOCK_SECTION, provideClockSkew } from './skew.plugin';
import { at } from '../../testing/must';

/** The tiny dataset where source 1 shares correlation ids with source 0, 2.5 s behind. */
function correlatedDataset(): MockDataset {
  const d = tinyDataset();
  for (let i = 0; i < 12; i++) {
    const fields = { CorrelationId: `k${i}`, MachineName: 'X' };
    d.records.push(
      { id: 0, ts: T0 + 600_000 + i * 1000, level: 'Information', message: `A ${i}`, template: null, exception: null, fields, source: 0, line: 100 + i, markers: [] },
      { id: 0, ts: T0 + 600_000 + i * 1000 - 2500, level: 'Information', message: `B ${i}`, template: null, exception: null, fields, source: 1, line: 100 + i, markers: [] },
    );
  }
  return d;
}

function setup(dataset: () => MockDataset = tinyDataset, extra: unknown[] = []) {
  const backend = new MockBackend({ latency: false, dataset, pluginMarkerKinds: [] });
  TestBed.configureTestingModule({
    providers: [
      provideTinyBackend(backend),
      provideFakeDock(),
      provideClockSkew(),
      { provide: Platform, useValue: { isTauri: () => false, pickPaths: async () => [], onFilesDropped: async () => () => undefined } },
      ...(extra as never[]),
    ],
  });
  const store = TestBed.inject(WorkspaceStore);
  return { backend, store, skew: TestBed.inject(ClockSkewStore) };
}

async function rows(backend: MockBackend): Promise<RecordView[]> {
  return (await backend.queryPage({ query: EMPTY_QUERY, offset: 0, limit: 1000, descending: false })).rows;
}

describe('skew formatting', () => {
  it('formats offsets, confidences and parses what people type', () => {
    expect(formatOffset(2350)).toBe('+2.35 s');
    expect(formatOffset(-3_723_500)).toBe('-1 h 2 min 3.5 s');
    expect(formatOffset(86_400_000)).toBe('+1 d');
    expect(formatOffset(0)).toBe('0 s');
    expect(formatOffset(Number.NaN)).toBe('0 s');
    expect(parseOffset('2.35')).toBe(2350);
    expect(parseOffset('-1.5')).toBe(-1500);
    expect(parseOffset('+2350ms')).toBe(2350);
    expect(parseOffset('-1h 5s')).toBe(-3_605_000);
    expect(parseOffset('2 min 3.5 s')).toBe(123_500);
    expect(parseOffset('1d')).toBe(86_400_000);
    expect(parseOffset('')).toBeNull();
    expect(parseOffset('abc')).toBeNull();
    expect(parseOffset('5s banana')).toBeNull();
    expect(formatConfidence(0.856)).toBe('86 %');
    expect(formatConfidence(2)).toBe('100 %');
    expect([confidenceLabel(0.9), confidenceLabel(0.6), confidenceLabel(0.1)]).toEqual(['high', 'medium', 'low']);
  });
});

describe('ClockSkewStore', () => {
  it('applies an offset, refreshes the summary and keeps selection and reference on their records', async () => {
    const { backend, store, skew } = setup();
    await settle();
    const all0 = await rows(backend);
    const worker = all0.find((r) => r.message === 'Application starting — Worker')!;
    const web01 = all0.find((r) => r.message === 'Slow request /api took 1500 ms')!;
    store.select(worker);
    expect(skew.setReference(web01)).toBe(true);
    expect(skew.setReference(all0.find((r) => r.ts === null)!)).toBe(false);
    expect(skew.setReference(null)).toBe(false);
    const jump = vi.spyOn(store, 'jumpTo');

    const change = await skew.apply({ kind: 'manual', source: 1, offsetMs: 200_000 });
    expect(change?.offsetMs).toBe(200_000);
    expect(store.summary()?.sources.find((s) => s.id === 1)?.offsetMs).toBe(200_000);
    const moved = (await rows(backend)).find((r) => r.message === 'Application starting — Worker')!;
    expect(jump).toHaveBeenCalledWith(moved.id);
    expect(skew.reference()?.id).toBe((await rows(backend)).find((r) => r.message === web01.message)!.id);
    expect(store.notice()).toContain('+3 min 20 s');

    // Moving the reference's own source updates its time too.
    await skew.apply({ kind: 'manual', source: 0, offsetMs: 1000 });
    expect(skew.reference()?.ts).toBe(web01.ts! + 1000);

    // Failures are shown, nothing else changes.
    expect(await skew.apply({ kind: 'manual', source: 1, offsetMs: 1e15 })).toBeNull();
    expect(skew.error()).toMatch(/too large/);
    skew.clearError();
    skew.clearReference();
    expect(skew.reference()).toBeNull();
  });

  it('drops a reference whose record no longer exists and reports suggestion failures', async () => {
    const { skew } = setup();
    await settle();
    skew.setReference({ id: 999, ts: 1, tsState: 'parsed', source: 0, sourceName: 'x', message: 'gone', level: 'Information', template: null, exception: null, fields: {}, line: 1, byteOffset: 0, group: '', markers: [] });
    await skew.apply({ kind: 'manual', source: 1, offsetMs: 10 });
    expect(skew.reference()).toBeNull();
    const backend = TestBed.inject(SkewBackend);
    vi.spyOn(backend, 'suggestOffsets').mockRejectedValueOnce('engine unavailable');
    await skew.suggest();
    expect(skew.error()).toBe('engine unavailable');
    expect(skew.suggesting()).toBe(false);
  });

  it('says calmly that clock correction is not available yet when the log store does not offer it', async () => {
    const refused = mockUnsupported('clock-skew correction is not available here', 'Use a different approach.');
    const { skew } = setup(tinyDataset, [{ provide: SkewBackend, useValue: { setSourceOffset: () => Promise.reject(refused), suggestOffsets: () => Promise.reject(refused) } }]);
    await settle();
    await skew.suggest();
    expect(skew.unavailable()).toBe('Clock-skew correction is not available here. Use a different approach.');
    expect(skew.error()).toBeNull();
    expect(await skew.apply({ kind: 'manual', source: 1, offsetMs: 10 })).toBeNull();
    expect(skew.unavailable()).toContain('not available');
    expect(skew.busy()).toBe(false);
  });
});

describe('ClockDialog', () => {
  function open(sourceId: number, dataset?: () => MockDataset, extra: unknown[] = []) {
    const close = vi.fn();
    const ctx = setup(dataset, [{ provide: DIALOG_REF, useValue: { close, closed: Promise.resolve() } }, ...extra]);
    const fixture = TestBed.createComponent(ClockDialog);
    fixture.componentRef.setInput('sourceId', sourceId);
    return { ...ctx, fixture, close };
  }

  it('applies a manual offset and validates the input', async () => {
    const { fixture, store, close } = open(1);
    await settle(fixture);
    expect(text(fixture)).toContain('app.log');
    expect(el(fixture, '[data-testid=current-offset]').textContent).toBe('0 s');
    const input = el<HTMLInputElement>(fixture, '[data-testid=offset-input]');
    typeInto(input, 'banana');
    await settle(fixture);
    expect(input.getAttribute('aria-invalid')).toBe('true');
    expect(text(fixture)).toContain('Not an offset');
    expect(buttonByText(fixture, 'Apply').disabled).toBe(true);
    typeInto(input, '+2.5');
    await settle(fixture);
    expect(text(fixture)).toContain('Will shift by +2.5 s');
    buttonByText(fixture, 'Apply').click();
    await settle(fixture);
    expect(store.summary()?.sources.find((s) => s.id === 1)?.offsetMs).toBe(2500);
    expect(close).toHaveBeenCalled();
    expect(text(fixture)).toContain('Reset to 0');
    buttonByText(fixture, 'Reset to 0').click();
    await settle(fixture);
    expect(store.summary()?.sources.find((s) => s.id === 1)?.offsetMs).toBe(0);
  });

  it('aligns the selected event on the reference', async () => {
    const { fixture, store, backend, skew } = open(1);
    await settle(fixture);
    expect(text(fixture)).toContain('Use as clock reference');
    const r = await rows(backend);
    skew.setReference(r.find((x) => x.message === 'Slow request /api took 1500 ms')!);
    await settle(fixture);
    expect(text(fixture)).toContain('Select the matching event of app.log');
    store.select(r.find((x) => x.message === 'Application starting — Worker')!);
    await settle(fixture);
    el(fixture, '[data-testid=align]').click();
    await settle(fixture);
    // Reference at T0+120 s, worker at T0+30 s: +90 s.
    expect(store.summary()?.sources.find((s) => s.id === 1)?.offsetMs).toBe(90_000);
    // Applying with nothing to align does nothing.
    skew.clearReference();
    await settle(fixture);
    expect(all(fixture, '[data-testid=align]')).toHaveLength(0);
  });

  it('shows "not available yet" as a status note, not an alert, when the log store does not offer it', async () => {
    const refused = mockUnsupported('clock-skew correction is not available here');
    const { fixture } = open(1, undefined, [{ provide: SkewBackend, useValue: { setSourceOffset: () => Promise.reject(refused), suggestOffsets: () => Promise.reject(refused) } }]);
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[data-testid=suggest]').click();
    await settle(fixture);
    expect(text(fixture)).toContain('Clock-skew correction is not available here.');
    expect(all(fixture, '[role=alert]')).toHaveLength(0);
  });

  it('shows the suggestion with its confidence and applies it', async () => {
    const { fixture, store } = open(1, correlatedDataset);
    await settle(fixture);
    el(fixture, '[data-testid=suggest]').click();
    await settle(fixture);
    const s = el(fixture, '[data-testid=suggestion]');
    expect(s.textContent).toContain('+2.5 s');
    expect(s.textContent).toContain('high confidence · 100 %');
    expect(s.textContent).toContain('12 of 12 events');
    el(fixture, '[data-testid=apply-suggestion]').click();
    await settle(fixture);
    expect(store.summary()?.sources.find((s) => s.id === 1)?.offsetMs).toBe(2500);
  });

  it('explains when there is nothing to suggest or the source is the reference', async () => {
    const a = open(0, correlatedDataset);
    await settle(a.fixture);
    el(a.fixture, '[data-testid=suggest]').click();
    await settle(a.fixture);
    expect(text(a.fixture)).toContain('This source is the reference');
    TestBed.resetTestingModule();
    const b = open(1);
    await settle(b.fixture);
    el(b.fixture, '[data-testid=suggest]').click();
    await settle(b.fixture);
    expect(text(b.fixture)).toContain('Too few events share');
    expect(text(b.fixture)).toContain('Use as clock reference');
  });
});

describe('clock section, commands, sidebar and inspector', () => {
  it('marks the reference and opens alignment from the inspector section', async () => {
    const { backend, skew } = setup();
    await settle();
    const r = await rows(backend);
    const fixture = TestBed.createComponent(ClockSection);
    fixture.componentRef.setInput('record', r[0]);
    await settle(fixture);
    buttonByText(fixture, 'Use as clock reference').click();
    await settle(fixture);
    expect(el(fixture, '[data-testid=is-reference]')).toBeTruthy();
    expect(skew.reference()?.id).toBe(at(r, 0).id);
    const other = r.find((x) => x.source !== at(r, 0).source && x.ts !== null)!;
    fixture.componentRef.setInput('record', other);
    await settle(fixture);
    const dialogs = TestBed.inject(DialogService);
    buttonByText(fixture, /Align app\.log on reference/).click();
    expect(dialogs.stack()[0]?.options.inputs).toEqual({ sourceId: other.source });
    expect(dialogs.stack()[0]?.options.title).toBe('Adjust clock — app.log');
    fixture.componentRef.setInput('record', r[0]);
    await settle(fixture);
    buttonByText(fixture, 'Clear').click();
    expect(skew.reference()).toBeNull();
    expect(CLOCK_SECTION.when!(at(r, 0))).toBe(true);
    expect(CLOCK_SECTION.when!(r.find((x) => x.ts === null)!)).toBe(false);
    TestBed.inject(ClockDialogService).open(3);
    expect(dialogs.stack()[1]?.options.title).toBe('Adjust clock');
  });

  it('offers palette commands for the selected event', async () => {
    const { backend, store, skew } = setup();
    await settle();
    const registry = TestBed.inject(CommandRegistry);
    const find = (id: string) => registry.all().find((c) => c.id === id)!;
    expect(registry.available().some((c) => c.id === 'skew.adjust')).toBe(false);
    const r = await rows(backend);
    store.select(at(r, 1));
    expect(registry.available().map((c) => c.id)).toEqual(expect.arrayContaining(['skew.adjust', 'skew.reference']));
    await registry.run(find('skew.reference').id);
    expect(skew.reference()?.id).toBe(at(r, 1).id);
    await registry.run(find('skew.adjust').id);
    expect(TestBed.inject(DialogService).stack()).toHaveLength(1);
    store.select(null);
    await find('skew.adjust').run(null as never);
    expect(TestBed.inject(DialogService).stack()).toHaveLength(1);
  });

  it('shows a skew badge and an Adjust clock entry on sources', async () => {
    const { store, skew } = setup();
    await settle();
    const fixture = TestBed.createComponent(Sidebar);
    await settle(fixture);
    expect(all(fixture, '[data-testid=skew-badge]')).toHaveLength(0);
    await skew.apply({ kind: 'manual', source: 1, offsetMs: -1500 });
    await settle(fixture);
    const badge = el(fixture, '[data-testid=skew-badge]');
    expect(badge.textContent?.trim()).toBe('-1.5 s');
    expect(badge.getAttribute('aria-label')).toBe('Clock shifted by -1.5 s');
    const row = badge.closest('.src') as HTMLElement;
    const menu = TestBed.inject(ContextMenuService);
    row.dispatchEvent(new MouseEvent('contextmenu', { clientX: 10, clientY: 20, bubbles: true }));
    expect(menu.menu()?.items.map((i) => i.label)).toEqual(['Open file', 'Show in folder', 'Parse with…', 'Add note…', 'Adjust clock…', 'Remove']);
    at(menu.menu()!.items, 4).run();
    const dialogs = TestBed.inject(DialogService);
    expect(dialogs.stack()[0]?.options.inputs).toEqual({ sourceId: 1 });
    el(fixture, '[aria-label="Adjust clock of app.clef"]').click();
    expect(dialogs.stack()).toHaveLength(2);
    at(menu.menu()!.items, 5).run();
    await settle(fixture);
    expect(store.summary()?.sources.some((s) => s.id === 1)).toBe(false);
  });

  it('shows the original timestamp of a shifted event in the inspector', async () => {
    const { backend, store, skew } = setup();
    await settle();
    await skew.apply({ kind: 'manual', source: 1, offsetMs: 2350 });
    const moved = (await rows(backend)).find((r) => r.message === 'Application starting — Worker')!;
    store.select(moved);
    const fixture = TestBed.createComponent(Inspector);
    await settle(fixture);
    const note = el(fixture, '[data-testid=skew-note]');
    expect(note.textContent).toContain('Shifted by +2.35 s');
    expect(note.textContent).toContain('09:00:30.000');
    store.select((await rows(backend)).find((r) => r.source === 0)!);
    await settle(fixture);
    expect(all(fixture, '[data-testid=skew-note]')).toHaveLength(0);
  });
});
