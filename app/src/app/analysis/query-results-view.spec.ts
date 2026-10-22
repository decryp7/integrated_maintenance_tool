import { ComponentFixture, TestBed } from '@angular/core/testing';
import { all, buttonByText, el, settle, text } from '../../testing/helpers';
import { QueryResultsView } from './query-results-view';
import { QuerySession } from './query-session';
import { loadWorkspace, provideAnalysisTesting } from './test-setup';
import { at } from '../../testing/must';

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

  it('greys the table out with a banner when it no longer answers the text in the bar', () => {
    session.table.set({ kind: 'table', columns: ['n'], rows: [[1]] });
    session.tableText.set('stats count()');
    session.text.set('stats count()');
    fixture.detectChanges();
    expect(all(fixture, '[data-testid=query-results-stale]')).toHaveLength(0);
    expect(el(fixture, '.body').classList.contains('stale')).toBe(false);

    session.error.set({ message: 'expected a value', offset: 9, len: 0 });
    fixture.detectChanges();
    expect(el(fixture, '[data-testid=query-results-stale]').textContent).toContain('fix the error and run again');
    expect(el(fixture, '.body').classList.contains('stale')).toBe(true);

    session.error.set(null);
    session.text.set('stats count() by level');
    fixture.detectChanges();
    expect(el(fixture, '[data-testid=query-results-stale]').textContent).toContain('run the query again');

    session.runError.set('IPC down');
    fixture.detectChanges();
    expect(el(fixture, '[data-testid=query-results-stale]').textContent).toContain('fix the error');
  });

  it('shows the table, sorts by column and exports CSV', async () => {
    session.table.set({ kind: 'table', columns: ['source', 'n'], rows: [['b.log', 5], ['a.log', 20], ['c.log', null]] });
    fixture.detectChanges();
    expect(text(fixture)).toContain('3 rows · 2 columns');
    const cells = () => all(fixture, 'tbody tr').map((r) => [...r.querySelectorAll('td')].map((c) => c.textContent).join(' ').trim());
    expect(cells()).toEqual(['b.log 5', 'a.log 20', 'c.log']);
    const headers = all<HTMLElement>(fixture, 'th');
    expect(headers[1]?.getAttribute('aria-sort')).toBeNull();
    at(all<HTMLButtonElement>(fixture, 'th button'), 1).click();
    fixture.detectChanges();
    expect(cells()).toEqual(['b.log 5', 'a.log 20', 'c.log']);
    expect(all(fixture, 'th')[1]?.getAttribute('aria-sort')).toBe('ascending');
    at(all<HTMLButtonElement>(fixture, 'th button'), 1).click();
    fixture.detectChanges();
    expect(cells()[0]).toBe('a.log 20');
    expect(all(fixture, 'th')[1]?.getAttribute('aria-sort')).toBe('descending');
    expect(all(fixture, 'th')[1]?.textContent).toContain('▼');
    at(all<HTMLButtonElement>(fixture, 'th button'), 0).click();
    fixture.detectChanges();
    expect(cells()[0]).toBe('a.log 20');
    expect(all(fixture, 'th')[0]?.textContent).toContain('▲');
    expect(el(fixture, 'td.num').textContent).toBe('20');

    const blobs: Blob[] = [];
    vi.stubGlobal('URL', { createObjectURL: (b: Blob) => (blobs.push(b), 'blob:x'), revokeObjectURL: () => undefined });
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined);
    buttonByText(fixture, 'Export CSV').click();
    expect(click).toHaveBeenCalled();
    const csv = await new Promise<string>((res) => {
      const r = new FileReader();
      r.onload = () => res(String(r.result));
      r.readAsText(at(blobs, 0));
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
