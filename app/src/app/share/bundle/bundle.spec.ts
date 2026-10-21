import { TestBed } from '@angular/core/testing';
import { buttonByText, settle, text } from '../../../testing/helpers';
import { BundleManifest } from '../../core/api.types';
import { Platform } from '../../core/platform';
import { WorkspaceStore } from '../../core/workspace.store';
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

const MANIFEST: BundleManifest = { format: 'imt-share', schema: 1, name: 'Case 42', createdAt: 1, imtVersion: '0.1.0', content: 'raw', redacted: true, redaction: { email: 3 }, events: null, entries: [], skipped: [] };

describe('bundle helpers', () => {
  it('recognises bundles', () => {
    expect(isBundlePath('/x/Case.IMTZ')).toBe(true);
    expect(isBundlePath('/x/case.zip')).toBe(false);
  });
});

describe('BundleService', () => {
  it('opens a bundle read-only with its UI state and reports, then saves it as a project or closes it', async () => {
    const { service, share, store, projects, reports } = await setup();
    const report = { version: 1, id: 'r1', title: 'Outage', summary: '', createdAt: 1, updatedAt: 1, sections: [] };
    share.addBundle('/out/case.imtz', { name: 'Case 42', ui: { version: 1 }, reports: [report], manifest: MANIFEST });
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
    expect(mockTools().calls).toContain('saveProject:/p/saved.imtz');
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

  it('shares by saving the project (no bundle dialog) and opens old bundles from the dialog and the commands', async () => {
    const { service, projects } = await setup();
    const commands = TestBed.inject(BundleCommands).commands;
    const share = at(commands, 0);
    const open = at(commands, 1);
    expect(share.title).toBe('Share as project file…');
    pickOpenPath.mockResolvedValueOnce(null);
    await open.run({} as never);
    expect(service.opened()).toBeNull();
    pickOpenPath.mockResolvedValueOnce('/demo/shared.imtz');
    await open.run({} as never);
    expect(pickOpenPath).toHaveBeenCalledWith('Open bundle', [{ name: 'IMT bundle', extensions: ['imtz'] }]);
    expect(service.opened()?.name).toBe('shared');
    expect(share.when!({ summary: TestBed.inject(WorkspaceStore).summary() } as never)).toBe(true);
    expect(share.when!({ summary: null } as never)).toBe(false);
    const saveAs = vi.spyOn(projects, 'saveAs').mockResolvedValueOnce(true);
    await share.run({} as never);
    expect(saveAs).toHaveBeenCalledTimes(1);
    expect(TestBed.inject(DialogService).stack()).toHaveLength(0);
  });

  it('is not what OpenRequests uses for .imtz files: those are projects now', async () => {
    const { service, projects } = await setup();
    const open = vi.spyOn(projects, 'open').mockResolvedValue(true);
    await TestBed.inject(OpenRequests).handle(['/demo/case.imtz', '/logs/x.log']);
    expect(open).toHaveBeenCalledWith('/demo/case.imtz', { session: false });
    expect(service.opened()).toBeNull();
    expect(TestBed.inject(WorkspaceStore).summary()?.sources.some((s) => s.path === '/logs/x.log')).toBe(true);
  });
});
