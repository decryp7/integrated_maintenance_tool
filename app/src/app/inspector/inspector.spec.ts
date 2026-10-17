import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text } from '../../testing/helpers';
import { ImtBackend } from '../core/backend';
import { EMPTY_QUERY, WorkspaceStore } from '../core/workspace.store';
import { Inspector } from './inspector';

describe('Inspector', () => {
  async function render(id: number) {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const fixture = TestBed.createComponent(Inspector);
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture);
    const page = await TestBed.inject(ImtBackend).queryPage({ query: { ...EMPTY_QUERY, groupBy: ['MachineName', 'ProcessId'] }, offset: id, limit: 1, descending: false });
    store.select(page.rows[0]);
    await settle(fixture);
    return { fixture, store };
  }

  it('shows an empty hint without a selection', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const fixture = TestBed.createComponent(Inspector);
    await settle(fixture);
    expect(text(fixture)).toContain('Select an event');
  });

  it('shows the event with badges, template holes, properties and exception', async () => {
    const { fixture } = await render(5);
    expect(text(fixture)).toContain('Host terminated unexpectedly');
    expect(text(fixture)).toContain('14 Oct 2026');
    expect(all(fixture, '.badge-mk').map((b) => b.textContent?.trim())).toEqual(['Fatal', 'Stop', 'Exception']);
    expect(el(fixture, '.exc').textContent).toContain('TimeoutException');
    expect(text(fixture)).toContain('app.clef:6');
    expect(all(fixture, '.prop .k').map((k) => k.textContent)).toEqual(['Application', 'MachineName', 'ProcessId']);
  });

  it('highlights template holes', async () => {
    const { fixture } = await render(0);
    expect(all(fixture, '.hole').map((h) => h.textContent)).toEqual(['{Application}', '{Version}', '{Framework}']);
  });

  it('adds filters from properties', async () => {
    const { fixture, store } = await render(5);
    all<HTMLButtonElement>(fixture, '.prop .icon-btn')[0].click();
    all<HTMLButtonElement>(fixture, '.prop .icon-btn')[3].click();
    expect(store.query().filters).toEqual([
      { field: 'Application', op: 'eq', value: 'Svc' },
      { field: 'MachineName', op: 'ne', value: 'WEB01' },
    ]);
  });

  it('shows context and correlated events', async () => {
    const { fixture, store } = await render(3);
    buttonByText(fixture, 'Show ±2 min').click();
    expect(store.query().from).toBe(store.selected()!.ts! - 120_000);
    expect(store.anchor()?.id).toBe(3);
    buttonByText(fixture, 'Same CorrelationId').click();
    expect(store.hasRange()).toBe(false);
    expect(store.query().filters).toContainEqual({ field: 'CorrelationId', op: 'eq', value: 'c1' });
  });

  it('copies JSON and reports clipboard failures', async () => {
    const { fixture, store } = await render(1);
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    buttonByText(fixture, 'Copy JSON').click();
    await settle(fixture);
    expect(JSON.parse(writeText.mock.calls[0][0]).id).toBe(1);
    expect(buttonByText(fixture, 'Copied')).toBeTruthy();
    writeText.mockRejectedValue(new Error('denied'));
    await new Promise((r) => setTimeout(r, 1600));
    buttonByText(fixture, 'Copy JSON').click();
    await settle(fixture);
    expect(store.error()).toMatch(/Clipboard/);
  });

  it('closes', async () => {
    const { fixture, store } = await render(1);
    el<HTMLButtonElement>(fixture, '[aria-label="Close inspector"]').click();
    expect(store.inspectorOpen()).toBe(false);
  });
});
