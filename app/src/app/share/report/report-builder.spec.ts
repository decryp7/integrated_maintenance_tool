import { Provider } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { AnalysisBackend } from '../../core/analysis-backend';
import { Anomaly, ChartDef, fullCoverage } from '../../core/api.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { ProfileEdits } from '../../profile/profile-edits';
import { QuerySession } from '../../analysis/query-session';
import { loadWorkspace } from '../../analysis/test-setup';
import { NotesBackend } from '../../notes/notes-backend';
import { MockNotesBackend } from '../../notes/mock/mock-notes-backend';
import { NotesStore } from '../../notes/notes.store';
import { TimelineAnnotations } from '../../timeline/annotations.store';
import { DIALOG_REF, DialogService } from '../../ui/dialog.service';
import { all, buttonByText, el, settle, text, typeInto } from '../../../testing/helpers';
import { MockShareBackend } from '../mock/mock-share-backend';
import { ShareBackend } from '../share-backend';
import { setupVisuals } from '../visuals/visual-testing';
import { MarkdownView } from './markdown-view';
import { anomalySeverity, MAX_FINDINGS, ReportActions } from './report-actions';
import { ReportBlockView, svgImageUrl } from './report-block';
import { openReportExport, ReportExportDialog } from './report-export-dialog';
import { blocksOf } from './report-model';
import { ReportCommands } from './report.commands';
import { REPORT_VIEWS } from './report.views';
import { ReportStore } from './report.store';
import { CommandRegistry } from '../../extensions/command-registry';
import { ReportView } from './report-view';
import { at } from '../../../testing/must';
import { mockJobs } from '../../jobs/mock-jobs';
import { NotificationsStore } from '../../notifications/notifications.store';

const def: ChartDef = { id: 'cpu', title: 'CPU %', y: 'avg(cpu)', bucket: '1m', splitBy: [], type: 'line', unit: '%' };
const ctx = () => {
  const shown: string[] = [];
  return { shown, c: { showView: (id: string) => shown.push(id) } as never };
};

async function setup(extra: Provider[] = []) {
  const v = setupVisuals(undefined, [{ provide: NotesBackend, useValue: new MockNotesBackend() }, ...extra]);
  const store = await loadWorkspace();
  await settle(undefined, 6);
  return { ...v, store, reports: TestBed.inject(ReportStore), actions: TestBed.inject(ReportActions) };
}

describe('ReportActions', () => {
  it('adds the selected event, notes, a chart, the timeline, a query table and annotations', async () => {
    const { store, reports, actions } = await setup();
    expect(actions.addSelectedEvent()).toBe(false);
    expect(actions.addNotes()).toBe(false);
    const page = await (TestBed.inject(ShareBackend) as MockShareBackend)['debrief'].queryPage({ query: store.query(), offset: 0, limit: 3, descending: false, anchor: null });
    store.select(at(page.rows, 1));
    expect(actions.addSelectedEvent()).toBe(true);
    expect(store.notice()).toBe('Added the selected event to the report');
    const notes = TestBed.inject(NotesStore);
    const eventId = at(page.rows, 1).id;
    await notes.add({ anchor: { kind: 'event', eventId, ts: null, source: 's', excerpt: 'look' }, text: 'look here', severity: 'problem' });
    await notes.add({ anchor: { kind: 'range', from: 1, to: 2 }, text: 'window\nmore' });
    await notes.add({ anchor: { kind: 'instant', at: 5 }, text: 'moment' });
    await notes.add({ anchor: { kind: 'project' }, text: 'overall', severity: 'warning' });
    expect(actions.addNotes()).toBe(true);
    expect(store.notice()).toBe('Added 4 notes to the report');
    expect(await actions.addChart(def, { series: [] })).toBe(true);
    expect(await actions.addChart(def, null)).toBe(true);
    store.resetRange();
    await settle(undefined, 6);
    expect(actions.addTimeline()).toBe(true);
    expect(actions.addText('hi')).toBe(true);

    const session = TestBed.inject(QuerySession);
    expect(actions.addQueryTable()).toBe(false);
    session.table.set({ kind: 'table', columns: ['k', 'n'], rows: Array.from({ length: 250 }, (_, i) => [`k${i}`, i]) });
    session.text.set('| stats count() by k');
    expect(actions.addQueryTable()).toBe(true);

    const ann = TestBed.inject(TimelineAnnotations);
    ann.items.set([]);
    expect(actions.addAnnotations()).toBe(false);
    ann.items.set([{ annotator: 'a', owner: 'o', kind: 'k', from: 1, to: 2, instance: null, on: null, label: 'reboot', severity: 'info', evidenceQuery: store.query() }]);
    expect(actions.addAnnotations()).toBe(true);

    const blocks = blocksOf(reports.active()!);
    expect(blocks.map((b) => b.type)).toEqual(['events', 'finding', 'finding', 'finding', 'finding', 'events', 'annotations', 'chart', 'chart', 'timeline', 'text', 'table', 'annotations']);
    expect(blocks[0]).toMatchObject({ ids: [at(page.rows, 1).id] });
    expect(blocks[1]).toMatchObject({ severity: 'critical', title: 'Event: look', detail: 'look here' });
    expect(blocks[2]).toMatchObject({ severity: 'info', title: 'Time range: Time range', detail: 'window\nmore' });
    expect(blocks[4]).toMatchObject({ severity: 'warning', title: 'Project: The whole project' });
    expect(blocks[5]).toMatchObject({ caption: 'Noted events', ids: [eventId] });
    expect(blocks[6]).toMatchObject({ caption: 'Noted times', items: [{ from: 1, to: 2, label: 'window', kind: 'note' }, { from: 5, to: null, label: 'moment', kind: 'note' }] });
    expect(blocks[7]).toMatchObject({ title: 'CPU %', spec: def });
    expect((blocks[7] as { svg: string }).svg).toContain('<text>chart</text>');
    expect((blocks[8] as { svg: string | null }).svg).toBeNull();
    expect((blocks[9] as { svg: string }).svg).toContain('<svg');
    expect(blocks[11]).toMatchObject({ query: '| stats count() by k' });
    expect((blocks[11] as { rows: unknown[] }).rows).toHaveLength(200);
    expect(blocks[12]).toMatchObject({ items: [{ label: 'reboot', kind: 'k' }] });
  });

  it('keeps a chart without a snapshot when rendering fails', async () => {
    const { reports, actions, service } = await setup();
    vi.spyOn(service, 'chartSvg').mockRejectedValue(new Error('no renderer'));
    await actions.addChart(def, { series: [] });
    expect(blocksOf(reports.active()!)[0]).toMatchObject({ type: 'chart', svg: null });
  });

  it('adds anomalies as findings, most severe first, and reports failures', async () => {
    const { reports, actions, store } = await setup();
    const analysis = TestBed.inject(AnalysisBackend);
    const many: Anomaly[] = Array.from({ length: 12 }, (_, i) => ({ kind: i === 0 ? 'crash' : 'errorSpike', instance: i % 2 ? 'WEB01' : null, from: 0, to: 1000, score: i, message: `a${i}`, sampleIds: [] }));
    vi.spyOn(analysis, 'queryAnomalies').mockResolvedValueOnce({ result: many, coverage: fullCoverage(12) }).mockResolvedValueOnce({ result: [], coverage: fullCoverage(0) }).mockRejectedValueOnce('engine down');
    expect(await actions.addAnomalies()).toBe(true);
    const findings = blocksOf(reports.active()!);
    expect(findings).toHaveLength(MAX_FINDINGS);
    expect(findings[0]).toMatchObject({ type: 'finding', title: 'a11', severity: 'warning' });
    expect((findings[0] as { detail: string }).detail).toContain('errorSpike on WEB01');
    expect(await actions.addAnomalies()).toBe(false);
    expect(store.notice()).toBe('No anomalies in the current results');
    expect(await actions.addAnomalies()).toBe(false);
    expect(store.error()).toContain('engine down');
    expect(anomalySeverity({ kind: 'crash' } as Anomaly)).toBe('critical');
    expect(anomalySeverity({ kind: 'memoryGrowth' } as Anomaly)).toBe('warning');
    expect(anomalySeverity({ kind: 'silence' } as Anomaly)).toBe('info');
  });
});

describe('ReportCommands', () => {
  it('registers the view, enables commands by context and shows the report after adding', async () => {
    const { store, reports } = await setup();
    expect(REPORT_VIEWS[0]?.id).toBe('report');
    expect(await REPORT_VIEWS[0]?.component()).toBe(ReportView);
    const commands = TestBed.inject(ReportCommands).commands;
    const byId = (id: string) => commands.find((c) => c.id === id)!;
    const { shown, c } = ctx();
    expect(byId('report.addSelected').when!(c)).toBe(false);
    expect(byId('report.addNotes').when!(c)).toBe(false);
    expect(byId('report.addQueryTable').when!(c)).toBe(false);
    expect(byId('report.export').when!(c)).toBe(false);
    expect(byId('report.addTimeline').when!(c)).toBe(true);
    expect(byId('report.addAnomalies').when!(c)).toBe(true);
    TestBed.inject(TimelineAnnotations).items.set([]);
    expect(byId('report.addAnnotations').when!(c)).toBe(false);
    await byId('report.show').run(c);
    await byId('report.new').run(c);
    await byId('report.addTimeline').run(c);
    await byId('report.addSelected').run(c);
    await byId('report.addNotes').run(c);
    await byId('report.addQueryTable').run(c);
    await byId('report.addAnnotations').run(c);
    vi.spyOn(TestBed.inject(AnalysisBackend), 'queryAnomalies').mockResolvedValue({ result: [], coverage: fullCoverage(0) });
    await byId('report.addAnomalies').run(c);
    expect(shown).toEqual(['report', 'report', 'report']);
    expect(byId('report.export').when!(c)).toBe(true);
    const dialogs = TestBed.inject(DialogService);
    const running = byId('report.export').run(c);
    expect(dialogs.stack()[0]?.component).toBe(ReportExportDialog);
    dialogs.dismissTop();
    await running;
    expect(store.summary()).not.toBeNull();
    expect(reports.reports()).toHaveLength(1);
  });
});

describe('ReportView and ReportBlockView', () => {
  it('offers the incident summary next to New report', async () => {
    await setup();
    const run = vi.spyOn(TestBed.inject(CommandRegistry), 'run').mockResolvedValue(true);
    const f = TestBed.createComponent(ReportView);
    await settle(f);
    buttonByText(f, /Incident summary/).click();
    expect(run).toHaveBeenCalledWith('incident.build', 'button');
  });

  it('shows an empty state, creates and edits a report, sections and blocks', async () => {
    const { reports } = await setup();
    const f = TestBed.createComponent(ReportView);
    await settle(f);
    expect(text(f)).toContain('No report yet');
    buttonByText(f, /New report/).click();
    await settle(f);
    const r = reports.active()!;
    typeInto(el<HTMLInputElement>(f, 'input.title'), 'Outage', 'change');
    typeInto(el<HTMLTextAreaElement>(f, 'textarea.summary'), 'Payments **failed**', 'change');
    await settle(f);
    expect(reports.active()!.title).toBe('Outage');
    expect(el(f, 'debrief-markdown-view strong').textContent).toBe('failed');

    buttonByText(f, 'Text').click();
    buttonByText(f, 'Timeline').click();
    await settle(f);
    expect(all(f, 'debrief-report-block')).toHaveLength(2);
    typeInto(el<HTMLTextAreaElement>(f, 'debrief-report-block textarea'), '# Hello', 'change');
    await settle(f);
    expect(el(f, 'debrief-report-block debrief-markdown-view [role=heading]').textContent?.trim()).toBe('Hello');
    typeInto(el<HTMLInputElement>(f, 'debrief-report-block input.caption'), 'The whole day', 'change');
    await settle(f);
    expect(blocksOf(reports.active()!)[1]).toMatchObject({ caption: 'The whole day' });
    expect(el<HTMLImageElement>(f, 'debrief-report-block img').src).toContain('data:image/svg+xml');

    // Keyboard move: Alt+ArrowUp on the focused second block, focus follows it.
    const second = at(all<HTMLElement>(f, 'debrief-report-block'), 1);
    second.focus();
    second.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', altKey: true, bubbles: true }));
    // Focus is back on the moved block before the next key arrives (a later refocus drops a quick second Alt+↑).
    expect(document.activeElement?.getAttribute('data-block-id')).toBe(at(blocksOf(reports.active()!), 0).id);
    second.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true }));
    await settle(f);
    expect(blocksOf(reports.active()!).map((b) => b.type)).toEqual(['timeline', 'text']);
    expect(document.activeElement?.getAttribute('data-block-id')).toBe(at(blocksOf(reports.active()!), 0).id);

    // Buttons: move down, remove.
    at(all<HTMLButtonElement>(f, 'debrief-report-block button[aria-label^="Move down"]'), 0).click();
    await settle(f);
    expect(blocksOf(reports.active()!).map((b) => b.type)).toEqual(['text', 'timeline']);
    at(all<HTMLButtonElement>(f, 'debrief-report-block button[aria-label^="Move up"]'), 1).click();
    await settle(f);
    at(all<HTMLButtonElement>(f, 'debrief-report-block button[aria-label^="Remove"]'), 0).click();
    await settle(f);
    expect(blocksOf(reports.active()!).map((b) => b.type)).toEqual(['text']);

    // Sections.
    buttonByText(f, /Add section/).click();
    await settle(f);
    expect(text(f)).toContain('Blocks are added to the last section');
    const titles = all<HTMLInputElement>(f, 'input.stitle');
    typeInto(at(titles, 1), 'Cause', 'change');
    at(all<HTMLButtonElement>(f, 'button[aria-label^="Move section up"]'), 1).click();
    await settle(f);
    expect(reports.active()!.sections.map((s) => s.title)).toEqual(['Cause', 'Timeline']);
    at(all<HTMLButtonElement>(f, 'button[aria-label^="Move section down"]'), 0).click();
    at(all<HTMLButtonElement>(f, 'button[aria-label^="Remove section"]'), 0).click();
    await settle(f);
    expect(reports.active()!.sections).toHaveLength(1);

    // A second report shows the picker; delete it again.
    reports.create('Second');
    await settle(f);
    const select = el<HTMLSelectElement>(f, '.pick select');
    typeInto(select, r.id, 'change');
    await settle(f);
    expect(reports.active()!.id).toBe(r.id);
    buttonByText(f, 'Delete report').click();
    await settle(f);
    expect(reports.reports()).toHaveLength(1);

    // Export opens the dialog.
    buttonByText(f, 'Export…').click();
    expect(TestBed.inject(DialogService).stack()[0]?.component).toBe(ReportExportDialog);
  });

  it('renders every block type as text, never as markup', async () => {
    const { reports } = await setup();
    reports.add({ type: 'events', caption: 'cap', ids: [4, 5] });
    reports.add({ type: 'finding', severity: 'critical', title: '<b>Spike</b>', detail: 'detail <i>' });
    reports.add({ type: 'table', query: '| stats count()', columns: ['<c>'], rows: [...Array.from({ length: 51 }, () => ['<script>x</script>', null, 3])] });
    reports.add({ type: 'table', columns: ['c'], rows: [] });
    reports.add({ type: 'annotations', caption: '', items: [{ from: 0, label: 'reboot <x>' }] });
    reports.add({ type: 'chart', title: 'C', svg: null });
    const blocks = blocksOf(reports.active()!);
    reports.replace([{ ...reports.active()!, sections: [{ ...reports.active()!.sections[0], blocks: [...blocks, { id: 'future', type: 'hologram' } as never] }] }]);
    const f = TestBed.createComponent(ReportView);
    await settle(f);
    expect(all(f, 'script, b, i')).toHaveLength(0);
    expect(text(f)).toContain('<b>Spike</b>');
    expect(text(f)).toContain('… and 1 more rows');
    expect(text(f)).toContain('reboot <x>');
    expect(text(f)).toContain('No snapshot.');
    expect(text(f)).toContain('"hologram" block');
    const store = TestBed.inject(WorkspaceStore);
    const jump = vi.spyOn(store, 'jumpTo');
    el<HTMLButtonElement>(f, 'button[aria-label="Show event 5"]').click();
    expect(jump).toHaveBeenCalledWith(5);
    const labels = all(f, 'debrief-report-block').map((b) => b.getAttribute('aria-label'));
    expect(labels).toEqual(['Events block: cap', 'Finding block: <b>Spike</b>', 'Query result block', 'Query result block', 'Annotations block', 'Chart block: C', 'hologram block']);
    expect(svgImageUrl('<svg>#</svg>')).toBe('data:image/svg+xml;charset=utf-8,%3Csvg%3E%23%3C%2Fsvg%3E');
    expect(ReportBlockView).toBeDefined();
  });

  it('renders markdown with a placeholder when empty', async () => {
    setupVisuals();
    const f = TestBed.createComponent(MarkdownView);
    f.componentRef.setInput('placeholder', 'Nothing yet');
    await settle(f);
    expect(text(f)).toContain('Nothing yet');
    f.componentRef.setInput('text', '> q\n- a\n1. b\n```\nc\n```\n---\n`k` _e_ [l](https://x.io)');
    await settle(f);
    for (const sel of ['blockquote', 'ul li', 'ol li', 'pre code', 'hr', 'code', 'em', 'a[href="https://x.io"]']) expect(all(f, sel).length).toBeGreaterThan(0);
  });
});

describe('ReportExportDialog', () => {
  async function dialog(onExport = false) {
    const closed: unknown[] = [];
    const v = await setup([{ provide: DIALOG_REF, useValue: { close: (r?: unknown) => closed.push(r ?? 'closed'), closed: Promise.resolve(undefined) } }]);
    vi.spyOn(TestBed.inject(ProfileEdits), 'current').mockResolvedValue({ toml: '', model: { redact: { on_export: onExport } } as never });
    v.reports.add({ type: 'text', markdown: 'mail alice@example.com from 10.0.0.1' });
    v.reports.update(v.reports.active()!.id, { title: 'My report' });
    const f = TestBed.createComponent(ReportExportDialog);
    await settle(f, 6);
    return { ...v, f, closed };
  }

  it('previews redaction, exports HTML with the chosen theme and closes', async () => {
    const { f, closed, share, picks } = await dialog(true);
    const box = el<HTMLInputElement>(f, 'input[type=checkbox]');
    expect(box.checked).toBe(true);
    expect(el(f, '[aria-label="Redaction preview"]').textContent).toMatch(/In the report text: .*1 × email/);
    el<HTMLInputElement>(f, 'input[value=dark]').click();
    el<HTMLInputElement>(f, 'input[value=light]').click();
    el<HTMLInputElement>(f, 'input[value=dark]').click();
    const spy = vi.spyOn(share, 'exportReport');
    buttonByText(f, 'Export…').click();
    await settle(f, 6);
    expect(picks).toEqual(['my-report.html']);
    expect(spy.mock.calls[0]?.[0]).toMatchObject({ format: 'html', path: '/out/my-report.html', redact: true, theme: 'dark' });
    // Written by a job: the call returned its id at once, a toast says so, the dialog closes.
    const job = await at(spy.mock.results, 0).value;
    expect(job).toEqual(expect.any(Number));
    expect(at(TestBed.inject(NotificationsStore).toasts(), 0).title).toBe('Exporting the report to my-report.html — running in Jobs');
    expect(closed).toEqual(['closed']);
    const end = await mockJobs().wait(job);
    expect([end.state, end.result]).toEqual(['done', { type: 'file', path: '/out/my-report.html' }]);
    expect(end.message).toMatch(/^Report written \(\d+ values redacted\)\.$/);
  });

  it('exports PDF without redaction, and shows failures and rule errors inline', async () => {
    const { f, share, closed } = await dialog(false);
    const box = el<HTMLInputElement>(f, 'input[type=checkbox]');
    expect(box.checked).toBe(false);
    expect(all(f, '[aria-label="Redaction preview"]')).toHaveLength(0);
    typeInto(el<HTMLSelectElement>(f, '#rep-format'), 'pdf', 'change');
    vi.spyOn(share, 'exportReport').mockRejectedValueOnce('locked');
    buttonByText(f, 'Export…').click();
    await settle(f, 6);
    expect(el(f, '[role=alert]').textContent).toContain('locked');
    buttonByText(f, 'Export…').click();
    await settle(f, 6);
    expect(TestBed.inject(NotificationsStore).toasts().at(-1)?.title).toBe('Exporting the report to my-report.pdf — running in Jobs');
    expect(closed).toHaveLength(1);

    vi.spyOn(share, 'previewRedaction').mockRejectedValueOnce('bad rules').mockResolvedValueOnce({ before: '', after: '', hits: [{ rule: 'email', count: 0 }], error: null });
    box.click();
    await settle(f, 6);
    expect(text(f)).toContain('The redaction rules do not compile: bad rules');
    box.click();
    box.click();
    await settle(f, 6);
    expect(text(f)).toContain('Nothing in the report text matches the rules.');
    expect(text(f)).not.toContain('referenced event');
  });

  it('does nothing when the save dialog is cancelled, and mentions referenced events', async () => {
    TestBed.resetTestingModule();
    const v = setupVisuals(() => null, [{ provide: DIALOG_REF, useValue: { close: () => undefined, closed: Promise.resolve(undefined) } }]);
    await loadWorkspace();
    vi.spyOn(TestBed.inject(ProfileEdits), 'current').mockRejectedValue('no profile');
    TestBed.inject(ReportStore).add({ type: 'events', ids: [1, 2] });
    const f = TestBed.createComponent(ReportExportDialog);
    await settle(f, 6);
    el<HTMLInputElement>(f, 'input[type=checkbox]').click();
    await settle(f, 6);
    expect(text(f)).toContain('The 2 referenced events are redacted when exported.');
    const spy = vi.spyOn(v.share, 'exportReport');
    buttonByText(f, 'Export…').click();
    await settle(f, 6);
    expect(spy).not.toHaveBeenCalled();
    expect(typeof openReportExport).toBe('function');
  });
});
