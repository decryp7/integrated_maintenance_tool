import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text } from '../../testing/helpers';
import { RecordView } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { ContextMenuService } from '../ui/context-menu.service';
import { Inspector } from './inspector';

const record = (patch: Partial<RecordView> = {}): RecordView => ({
  id: 10,
  ts: null, tsState: 'untimed',
  level: 'Information',
  message: 'line without a timestamp',
  template: null,
  exception: null,
  fields: {},
  source: 1,
  sourceName: 'app.log',
  line: 3, byteOffset: 0,
  group: 'WEB02 · 300',
  markers: [],
  ...patch,
});

describe('Inspector context section and edge cases', () => {
  async function render(rec: RecordView) {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const fixture = TestBed.createComponent(Inspector);
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture);
    store.select(rec);
    await settle(fixture);
    return { fixture, store };
  }

  it('offers the Context section for the selected event and loads it on demand', async () => {
    const { fixture } = await render(record());
    expect(all(fixture, 'debrief-context-section')).toHaveLength(1);
    expect(all(fixture, '.line')).toHaveLength(0);
    el<HTMLButtonElement>(fixture, 'debrief-context-section button.toggle').click();
    await settle(fixture);
    expect(all(fixture, '.line.hit')).toHaveLength(1);
  });

  it('handles an event without a timestamp, properties, markers it does not know, or a plain template', async () => {
    const { fixture, store } = await render(record({ markers: ['mystery'], template: '{Who} did it', message: 'Bob did it' }));
    expect(text(fixture)).toContain('No properties.');
    expect(all(fixture, 'button').some((b) => b.textContent?.includes('Show ±2 min'))).toBe(false);
    expect(el(fixture, '.badge-mk').textContent).toContain('mystery');
    expect(all(fixture, '.hole').map((h) => h.textContent)).toEqual(['{Who}']);
    (fixture.componentInstance as unknown as { showContext(): void }).showContext();
    expect(store.anchor()).toBeNull();
    store.select(record({ template: 'plain text only', message: 'different' }));
    await settle(fixture);
    expect(all(fixture, '.hole')).toHaveLength(0);
    expect(el(fixture, '.template').textContent).toContain('plain text only');
  });

  it('has one More actions menu: Mask and Extract wait there, disabled with the reason, and values can break at dots (K1)', async () => {
    const { fixture } = await render(record({ message: 'plain', fields: { SourceContext: 'Microsoft.Hosting.Lifetime' } }));
    expect(all(fixture, '.actions button').map((b) => b.textContent?.trim())).not.toContain('Mask…');
    expect(all(fixture, '.actions button').map((b) => b.textContent?.trim())).toContain('More actions…');
    expect(el(fixture, '.prop .v').querySelectorAll('wbr').length).toBeGreaterThan(1);
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    el<HTMLButtonElement>(fixture, '[data-testid=prop-copy]').click();
    await settle(fixture);
    expect(writeText).toHaveBeenCalledWith('Microsoft.Hosting.Lifetime');
    buttonByText(fixture, 'More actions…').click();
    const items = TestBed.inject(ContextMenuService).menu()!.items;
    const mask = items.find((i) => i.label.startsWith('Mask'));
    expect(mask).toMatchObject({ disabled: true, hint: 'Select text in the message first' });
    expect(items.find((i) => i.label.startsWith('Extract'))?.disabled).toBe(true);
  });

  it('ignores actions when nothing is selected', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const fixture = TestBed.createComponent(Inspector);
    await settle(fixture);
    const inst = fixture.componentInstance as unknown as { copyJson(): Promise<void>; sameAs(f: string, v: string): void; mask(): void; extract(): void; openActions(e: Event): void; openPropMenu(e: Event, n: string, v: string): void; onSelectionChange(): void; showContext(): void };
    await inst.copyJson();
    inst.mask();
    inst.extract();
    inst.openActions(new MouseEvent('click'));
    inst.openPropMenu(new MouseEvent('contextmenu'), 'a', 'b');
    inst.onSelectionChange();
    inst.showContext();
    expect(TestBed.inject(WorkspaceStore).error()).toBeNull();
    expect(buttonByText).toBeDefined();
  });
});
