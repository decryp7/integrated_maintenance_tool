import { ComponentFixture, TestBed } from '@angular/core/testing';
import { all, buttonByText, el, settle, text } from '../../testing/helpers';
import { QueryResultsView } from './query-results-view';
import { QuerySession } from './query-session';
import { loadWorkspace, provideAnalysisTesting } from './test-setup';

describe('QueryResultsView', () => {
  let fixture: ComponentFixture<QueryResultsView>;
  let session: QuerySession;

  beforeEach(async () => {
    TestBed.configureTestingModule({ providers: provideAnalysisTesting() });
    await loadWorkspace();
    session = TestBed.inject(QuerySession);
    fixture = TestBed.createComponent(QueryResultsView);
    fixture.detectChanges();
  });

  it('explains how to get a table when there is none', () => {
    expect(text(fixture)).toContain('stats');
    expect(buttonByText(fixture, 'Export CSV').disabled).toBe(true);
  });

  it('shows the table, sorts by column and exports CSV', async () => {
    session.table.set({ kind: 'table', columns: ['source', 'n'], rows: [['b.log', 5], ['a.log', 20], ['c.log', null]] });
    fixture.detectChanges();
    expect(text(fixture)).toContain('3 rows · 2 columns');
    const cells = () => all(fixture, 'tbody tr').map((r) => [...r.querySelectorAll('td')].map((c) => c.textContent).join(' ').trim());
    expect(cells()).toEqual(['b.log 5', 'a.log 20', 'c.log']);
    const headers = all<HTMLElement>(fixture, 'th');
    expect(headers[1].getAttribute('aria-sort')).toBeNull();
    all<HTMLButtonElement>(fixture, 'th button')[1].click();
    fixture.detectChanges();
    expect(cells()).toEqual(['b.log 5', 'a.log 20', 'c.log']);
    expect(all(fixture, 'th')[1].getAttribute('aria-sort')).toBe('ascending');
    all<HTMLButtonElement>(fixture, 'th button')[1].click();
    fixture.detectChanges();
    expect(cells()[0]).toBe('a.log 20');
    expect(all(fixture, 'th')[1].getAttribute('aria-sort')).toBe('descending');
    expect(all(fixture, 'th')[1].textContent).toContain('▼');
    all<HTMLButtonElement>(fixture, 'th button')[0].click();
    fixture.detectChanges();
    expect(cells()[0]).toBe('a.log 20');
    expect(all(fixture, 'th')[0].textContent).toContain('▲');
    expect(el(fixture, 'td.num').textContent).toBe('20');

    const blobs: Blob[] = [];
    vi.stubGlobal('URL', { createObjectURL: (b: Blob) => (blobs.push(b), 'blob:x'), revokeObjectURL: () => undefined });
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined);
    buttonByText(fixture, 'Export CSV').click();
    expect(click).toHaveBeenCalled();
    const csv = await new Promise<string>((res) => {
      const r = new FileReader();
      r.onload = () => res(String(r.result));
      r.readAsText(blobs[0]);
    });
    expect(csv).toBe('source,n\r\na.log,20\r\nb.log,5\r\nc.log,\r\n');
    click.mockRestore();
    vi.unstubAllGlobals();
    session.table.set(null);
    fixture.detectChanges();
    await settle(fixture);
    expect(all(fixture, 'table')).toHaveLength(0);
    buttonByText(fixture, 'Export CSV');
  });
});
