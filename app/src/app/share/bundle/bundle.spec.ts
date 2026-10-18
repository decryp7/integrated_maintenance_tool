import { TestBed } from '@angular/core/testing';
import { buttonByText, el, settle, text, typeInto } from '../../../testing/helpers';
import { ImtBackend } from '../../core/backend';
import { Platform } from '../../core/platform';
import { WorkspaceStore } from '../../core/workspace.store';
import { UiStateRegistry } from '../../lifecycle/ui-state';
import { OpenRequests } from '../../projects/open-requests';
import { ProjectService } from '../../projects/project.service';
import { mockTools, provideToolsTesting } from '../../tools/mock/test-support';
import { DIALOG_REF, DialogService } from '../../ui/dialog.service';
import { MockShareBackend } from '../mock/mock-share-backend';
import { provideMockShare } from '../redaction/share-testing';
import { ReportStore } from '../report/report.store';
import { ShareBackend } from '../share-backend';
import { BundleBanner } from './bundle-banner';
import { BundleCommands } from './bundle.commands';
import { BundleDialog, bundleFileName, formatBytes } from './bundle-dialog';
import { BundleService, isBundlePath } from './bundle.service';
import { at } from '../../../testing/must';

const closed = vi.fn();
const pickSavePath = vi.fn<(title: string, name: string, filters: unknown) => Promise<string | null>>();
const pickOpenPath = vi.fn<(title: string, filters: unknown) => Promise<string | null>>();

async function setup() {
  closed.mockClear();
  pickSavePath.mockReset();
  pickOpenPath.mockReset();
  TestBed.configureTestingModule({
    providers: [
      ...provideToolsTesting(),
      provideMockShare(),
      { provide: DIALOG_REF, useValue: { close: closed, closed: Promise.resolve(undefined) } },
      { provide: Platform, useValue: { isTauri: () => false, pickPaths: async () => [], onFilesDropped: async () => () => undefined, pickSavePath, pickOpenPath } },
    ],
  });
  const store = TestBed.inject(WorkspaceStore);
  await settle();
  return { store, service: TestBed.inject(BundleService), share: TestBed.inject(ShareBackend) as MockShareBackend, projects: TestBed.inject(ProjectService), reports: TestBed.inject(ReportStore) };
}

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe('bundle helpers', () => {
  it('recognises bundles and formats sizes and names', () => {
    expect(isBundlePath('/x/Case.IMTZ')).toBe(true);
    expect(isBundlePath('/x/case.zip')).toBe(false);
    expect([formatBytes(12), formatBytes(2048), formatBytes(3 << 20), formatBytes(5 * 2 ** 30)]).toEqual(['12 B', '2 KB', '3.0 MB', '5.0 GB']);
    expect(bundleFileName('a/b: c')).toBe('a_b_ c.imtz');
    expect(bundleFileName(' ')).toBe('bundle.imtz');
  });
});

describe('BundleDialog', () => {
  async function dialog() {
    const ctx = await setup();
    ctx.reports.create('Outage');
    const fixture = TestBed.createComponent(BundleDialog);
    await settle(fixture);
    await wait(250);
    await settle(fixture);
    return { ...ctx, fixture };
  }

  it('estimates the bundle live as the options change', async () => {
    const { fixture } = await dialog();
    expect(text(fixture)).toMatch(/About .* before compression · 6 files · 11 events/);
    el<HTMLInputElement>(fixture, 'input[value=events]').click();
    await wait(250);
    await settle(fixture);
    expect(text(fixture)).toMatch(/· 5 files · 11 events/);
    el<HTMLInputElement>(fixture, 'input[value=raw]').click();
    const boxes = [...fixture.nativeElement.querySelectorAll('input[type=checkbox]')] as HTMLInputElement[];
    const redact = at(boxes, 0);
    const reports = at(boxes, 1);
    redact.click();
    reports.click();
    await wait(250);
    await settle(fixture);
    expect(text(fixture)).toContain('Binary files and archives cannot be redacted');
    expect(text(fixture)).toMatch(/· 5 files ·/);
    vi.spyOn(TestBed.inject(ShareBackend), 'estimateBundle').mockResolvedValueOnce({ files: 3, bytes: 10, events: 0, parts: [], skipped: [{ path: '/logs/a.evtx', reason: 'binary files cannot be redacted' }] });
    el<HTMLInputElement>(fixture, 'input[value=events]').click();
    await wait(250);
    await settle(fixture);
    expect(text(fixture)).toContain('a.evtx: binary files cannot be redacted');
  });

  it('writes the bundle where the user chose and reports it', async () => {
    const { fixture, share, store } = await dialog();
    const spy = vi.spyOn(share, 'createBundle');
    typeInto(el<HTMLInputElement>(fixture, 'input[aria-label="Bundle name"]'), 'Case 42');
    (fixture.nativeElement.querySelectorAll('input[type=checkbox]')[0] as HTMLInputElement).click();
    await settle(fixture);
    pickSavePath.mockResolvedValueOnce(null);
    buttonByText(fixture, 'Save bundle…').click();
    await settle(fixture);
    expect(pickSavePath).toHaveBeenCalledWith('Share bundle', 'Case 42.imtz', [{ name: 'IMT bundle', extensions: ['imtz'] }]);
    expect(spy).not.toHaveBeenCalled();
    pickSavePath.mockResolvedValueOnce('/out/case.imtz');
    buttonByText(fixture, 'Save bundle…').click();
    await settle(fixture);
    expect(spy.mock.calls[0]?.[0]).toMatchObject({ name: 'Case 42', content: 'raw', redact: true, reports: [expect.objectContaining({ title: 'Outage' })] });
    expect(spy.mock.calls[0]?.[0].ui).toEqual(TestBed.inject(UiStateRegistry).serialize());
    expect(store.notice()).toBe('Bundle written to case.imtz, redacted (5 values)');
    expect(closed).toHaveBeenCalled();
  });

  it('shows failures and lets the user cancel', async () => {
    const { fixture, share, store } = await dialog();
    pickSavePath.mockResolvedValue('/out/x.imtz');
    const manifest = await share.createBundle({ name: 'n', content: 'raw', query: store.effectiveQuery(), redact: false, ui: null, reports: [] }, '/o/n.imtz');
    vi.spyOn(share, 'createBundle').mockRejectedValueOnce(new Error('disk full')).mockResolvedValueOnce(manifest);
    buttonByText(fixture, 'Save bundle…').click();
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('disk full');
    buttonByText(fixture, 'Save bundle…').click();
    await settle(fixture);
    expect(store.notice()).toBe('Bundle written to x.imtz');
    vi.spyOn(share, 'estimateBundle').mockRejectedValueOnce(new Error('cannot estimate'));
    el<HTMLInputElement>(fixture, 'input[value=events]').click();
    await wait(250);
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('cannot estimate');
    buttonByText(fixture, 'Cancel').click();
    expect(closed).toHaveBeenCalled();
  });

  it('defaults to the project name and the profile redaction default', async () => {
    const ctx = await setup();
    await TestBed.inject(ImtBackend).saveProfile((await TestBed.inject(ImtBackend).getProfile()).toml.replace('on_export = false', 'on_export = true'));
    ctx.projects.name.set('Incident');
    const fixture = TestBed.createComponent(BundleDialog);
    await settle(fixture);
    expect(el<HTMLInputElement>(fixture, 'input[aria-label="Bundle name"]').value).toBe('Incident');
    expect((fixture.nativeElement.querySelectorAll('input[type=checkbox]')[0] as HTMLInputElement).checked).toBe(true);
    expect((fixture.nativeElement.querySelectorAll('input[type=checkbox]')[1] as HTMLInputElement).disabled).toBe(true);
  });
});

describe('BundleService', () => {
  it('opens a bundle read-only with its UI state and reports, then saves it as a project or closes it', async () => {
    const { service, share, store, projects, reports } = await setup();
    const report = { version: 1, id: 'r1', title: 'Outage', summary: '', createdAt: 1, updatedAt: 1, sections: [] };
    await share.createBundle({ name: 'Case 42', content: 'raw', query: store.effectiveQuery(), redact: true, ui: { version: 1 }, reports: [report] }, '/out/case.imtz');
    expect(await service.open('/out/case.imtz')).toBe(true);
    expect(service.opened()).toMatchObject({ path: '/out/case.imtz', name: 'Case 42' });
    expect(reports.reports()).toEqual([report]);
    expect(projects.path()).toBeNull();
    expect(projects.name()).toBe('Case 42');
    expect(projects.dirty()).toBe(false);
    expect(store.notice()).toBe('Opened case.imtz (read-only)');

    const banner = TestBed.createComponent(BundleBanner);
    await settle(banner);
    expect(text(banner)).toMatch(/Opened from Case 42 \(read-only\) · redacted · created .* · IMT 0\.1\.0/);
    buttonByText(banner, 'Save as project…').click();
    await settle(banner);
    expect(mockTools().calls).toContain('saveProject:/p/saved.imtproj');
    expect(service.opened()).toBeNull();

    await service.open('/demo/shared.imtz');
    await settle(banner);
    expect(text(banner)).toContain('Opened from shared (read-only)');
    expect(text(banner)).not.toContain('·');
    buttonByText(banner, 'Close').click();
    await settle(banner);
    expect(service.opened()).toBeNull();
    expect(store.summary()?.total).toBe(0);
  });

  it('keeps the bundle state when Save as is cancelled and reports open failures', async () => {
    const { service, store } = await setup();
    await service.open('/demo/x.imtz');
    vi.spyOn(TestBed.inject(ProjectService), 'saveAs').mockResolvedValueOnce(false);
    await service.saveAsProject();
    expect(service.opened()).not.toBeNull();
    expect(await service.open('/demo/x.zip')).toBe(false);
    expect(store.error()).toContain('Could not open the bundle');
    expect(service.busy()).toBe(false);
  });

  it('opens from the dialog and the commands', async () => {
    const { service } = await setup();
    const commands = TestBed.inject(BundleCommands).commands;
    const create = at(commands, 0);
    const open = at(commands, 1);
    pickOpenPath.mockResolvedValueOnce(null);
    await open.run({} as never);
    expect(service.opened()).toBeNull();
    pickOpenPath.mockResolvedValueOnce('/demo/shared.imtz');
    await open.run({} as never);
    expect(pickOpenPath).toHaveBeenCalledWith('Open bundle', [{ name: 'IMT bundle', extensions: ['imtz'] }]);
    expect(service.opened()?.name).toBe('shared');
    expect(create.when!({ summary: TestBed.inject(WorkspaceStore).summary() } as never)).toBe(true);
    expect(create.when!({ summary: null } as never)).toBe(false);
    void create.run({} as never);
    await settle();
    expect(TestBed.inject(DialogService).stack()[0]?.options.title).toBe('Share as bundle');
  });

  it('is what OpenRequests uses for .imtz files', async () => {
    const { service } = await setup();
    await TestBed.inject(OpenRequests).handle(['/demo/shared.imtz', '/logs/x.log']);
    expect(service.opened()?.path).toBe('/demo/shared.imtz');
    expect(TestBed.inject(WorkspaceStore).summary()?.sources.some((s) => s.path === '/logs/x.log')).toBe(true);
  });
});
