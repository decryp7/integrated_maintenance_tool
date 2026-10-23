import { TestBed } from '@angular/core/testing';
import { MockAnalysisBackend, recordsFrom } from '../../analysis/mock/mock-analysis-backend';
import { CmdError } from '../../core/cmd-error';
import { RedactedCopyChoices, RedactedCopyPreview, RedactedExport } from '../../core/api.types';
import { MockDataset } from '../../core/mock/mock-data';
import { MockBackend } from '../../core/mock/mock-backend';
import { Platform } from '../../core/platform';
import { ProjectService } from '../../projects/project.service';
import { DialogService, DIALOG_REF } from '../../ui/dialog.service';
import { OverwriteGuard } from '../../ui/overwrite-guard';
import { all, el, provideTinyBackend, settle, tinyBackend, tinyDataset } from '../../../testing/helpers';
import { provideToolsTesting } from '../../tools/mock/test-support';
import { MockShareBackend } from '../mock/mock-share-backend';
import { ShareBackend, TauriShareBackend } from '../share-backend';
import { RedactedCopyCommands } from './redacted-copy-commands';
import { PREVIEW_DEBOUNCE_MS, RedactedCopyDialog } from './redacted-copy-dialog';
import { detectorLabel, modeChoices } from './redacted-copy-labels';

/** The tiny incident plus an email in one event and a CSV source. */
function dataset(): MockDataset {
  const d = tinyDataset();
  const records = d.records.map((r, i) => (i === 0 ? { ...r, message: 'login by ann@example.com from 10.1.2.3' } : r));
  return { ...d, records, sources: [...d.sources, { id: 2, path: '/logs/orders.csv', name: 'orders.csv', importer: 'csv', skipped: 0 }] };
}

function mockShare(): { debrief: MockBackend; share: MockShareBackend } {
  const debrief = tinyBackend({ dataset });
  return { debrief, share: new MockShareBackend(debrief, new MockAnalysisBackend(recordsFrom(debrief))) };
}

describe('TauriShareBackend redacted copy', () => {
  it('maps the three commands with their arguments', async () => {
    const call = vi.fn(async (...a: unknown[]) => (a[0] === 'jobs_wait' ? { state: 'done', result: { type: 'file', path: '/c.debriefz', detail: { sources: 1 } } } : a[0] === 'export_redacted_copy' ? 9 : null));
    const b = new TauriShareBackend(call as never);
    await b.redactedCopyChoices();
    await b.redactedCopyPreview({ detectors: ['email'], mode: 'hash' });
    expect(await b.exportRedactedCopy({ path: '/c.debriefz', name: 'c', options: { rules: [] }, ui: { a: 1 } })).toEqual({ sources: 1 });
    await b.exportRedactedCopy({ path: '/c.debriefz', name: 'c', options: {}, ui: null, reports: [], overwrite: true });
    expect(call.mock.calls).toEqual([
      ['redacted_copy_choices'],
      ['redacted_copy_preview', { options: { detectors: ['email'], mode: 'hash' } }],
      ['export_redacted_copy', { path: '/c.debriefz', name: 'c', options: { rules: [] }, ui: { a: 1 } }],
      ['jobs_wait', { id: 9 }],
      ['export_redacted_copy', { path: '/c.debriefz', name: 'c', options: {}, ui: null, reports: [], overwrite: true }],
      ['jobs_wait', { id: 9 }],
    ]);
  });
});

describe('MockShareBackend redacted copy', () => {
  it('offers all detectors and the profile selection', async () => {
    const { share } = mockShare();
    const c = await share.redactedCopyChoices();
    expect(c.detectors).toHaveLength(13);
    expect(c.selected.detectors).toContain('email');
    expect(c.selected.mode).toBe('mask');
  });

  it('counts only the picked detectors in text sources and lists the CSV as left out', async () => {
    const { share } = mockShare();
    const p = await share.redactedCopyPreview({ detectors: ['email'] });
    expect(p.included).toBe(2);
    expect(p.excluded).toEqual([{ name: 'orders.csv', reason: expect.stringContaining('csv') }]);
    expect(p.summary.hits).toEqual([{ rule: 'email', count: 1 }]);
    expect(p.summary.changed).toBe(1);
    expect(p.summary.samples[0]?.after).toContain('<email>');
    const both = await share.redactedCopyPreview({ detectors: ['email', 'ipv4'], mode: 'drop' });
    expect(both.summary.hits.map((h) => h.count)).toEqual([1, 1]);
    expect(both.summary.samples[0]?.after).toBe('login by  from ');
  });

  it('refuses when nothing is picked or a detector is unknown', async () => {
    const { share } = mockShare();
    await expect(share.redactedCopyPreview({ detectors: [], rules: [] })).rejects.toMatchObject({ code: 'invalid', field: 'options' });
    await expect(share.redactedCopyPreview({ detectors: ['nope'] })).rejects.toBeInstanceOf(CmdError);
  });

  it('writes the copy once; a second write needs overwrite', async () => {
    const { share } = mockShare();
    const req = { path: '/c.debriefz', name: 'c', options: { detectors: ['email'] }, ui: {} };
    const out = await share.exportRedactedCopy(req);
    expect(out).toMatchObject({ sources: 2, events: 11, excluded: [{ name: 'orders.csv' }], hits: [{ rule: 'email', count: 1 }] });
    expect(out.bytes).toBeGreaterThan(0);
    expect(share.written).toHaveLength(1);
    await expect(share.exportRedactedCopy(req)).rejects.toThrow('already exists');
    await expect(share.exportRedactedCopy({ ...req, overwrite: true })).resolves.toBeTruthy();
    await expect(share.exportRedactedCopy({ ...req, path: '/c.zip', overwrite: true })).rejects.toThrow('.debriefz');
  });
});

describe('labels', () => {
  it('names the known detectors and falls back to the id', () => {
    expect(detectorLabel('ipv4')).toBe('IPv4 addresses');
    expect(detectorLabel('something-new')).toBe('something-new');
    for (const id of ['bearer-token', 'jwt', 'api-key', 'connection-string', 'password', 'user-path', 'email', 'credit-card', 'ipv4', 'ipv6', 'guid', 'phone', 'hostname']) {
      expect(detectorLabel(id)).not.toBe(id);
    }
    expect(modeChoices().map((m) => m.mode)).toEqual(['mask', 'hash', 'drop']);
  });
});

describe('Share redacted copy command', () => {
  beforeEach(() => TestBed.configureTestingModule({ providers: [provideTinyBackend()] }));

  it('is registered in the File category, needs sources and opens the dialog', async () => {
    const cmd = TestBed.inject(RedactedCopyCommands).commands[0]!;
    expect(cmd).toMatchObject({ id: 'share.redactedCopy', title: 'Share redacted copy…', category: 'File' });
    expect(cmd.when?.({ summary: { sources: [] } } as never)).toBe(false);
    expect(cmd.when?.({ summary: { sources: [{}] } } as never)).toBe(true);
    await cmd.run({} as never);
    const open = TestBed.inject(DialogService).stack();
    expect(open).toHaveLength(1);
    expect(open[0]?.options.title).toBe('Share redacted copy');
  });
});

const CHOICES: RedactedCopyChoices = {
  detectors: ['email', 'ipv4', 'brand-new'],
  rules: ['order-ref'],
  selected: { detectors: ['email'], mode: 'hash', fields: [], onExport: false, rule: [{ name: 'order-ref', regex: 'x' }] },
};
const preview = (changed: number): RedactedCopyPreview => ({
  summary: { scanned: 10, changed, truncated: false, hits: [{ rule: 'email', count: changed }], samples: [{ recordId: 1, before: 'a@b.io', after: '<email:1a>' }] },
  excluded: [{ name: 'orders.csv', reason: 'CSV cannot be redacted' }],
  included: 2,
});
const RESULT: RedactedExport = { bytes: 2048, sources: 2, events: 11, excluded: [{ name: 'orders.csv', reason: 'CSV cannot be redacted' }], hits: [{ rule: 'email', count: 3 }] };

async function setup(patch: Partial<Record<'choices' | 'preview' | 'export', ReturnType<typeof vi.fn>>> = {}, path: string | null = '/out/c.debriefz') {
  const backend = {
    redactedCopyChoices: patch.choices ?? vi.fn(async () => CHOICES),
    redactedCopyPreview: patch.preview ?? vi.fn(async () => preview(3)),
    exportRedactedCopy: patch.export ?? vi.fn(async () => RESULT),
  };
  const ref = { close: vi.fn(), closed: Promise.resolve(undefined) };
  const pickSavePath = vi.fn(async (..._a: unknown[]) => path);
  TestBed.configureTestingModule({
    providers: [
      provideTinyBackend(),
      ...provideToolsTesting(),
      { provide: ShareBackend, useValue: backend },
      { provide: DIALOG_REF, useValue: ref },
    ],
  });
  const platform = TestBed.inject(Platform);
  platform.pickSavePath = pickSavePath as never;
  Object.assign(platform, { saveDialogConfirmsOverwrite: () => true });
  const fixture = TestBed.createComponent(RedactedCopyDialog);
  await settle(fixture);
  return { backend, ref, pickSavePath, fixture };
}

const wait = (ms = PREVIEW_DEBOUNCE_MS + 60) => new Promise((r) => setTimeout(r, ms));
const box = (f: Parameters<typeof el>[0], sel: string) => el<HTMLInputElement>(f, sel);
const createBtn = (f: Parameters<typeof el>[0]) => el<HTMLButtonElement>(f, '[data-testid=rc-create]');

describe('RedactedCopyDialog', () => {
  it('preselects what the profile selects and shows the first preview, left-out list and fixed notes', async () => {
    const { fixture, backend } = await setup();
    expect(box(fixture, '[data-detector=email]').checked).toBe(true);
    expect(box(fixture, '[data-detector=ipv4]').checked).toBe(false);
    expect(box(fixture, '[data-rule=order-ref]').checked).toBe(true);
    expect(box(fixture, '[data-mode=hash]').checked).toBe(true);
    expect(el(fixture, '[data-testid=rc-detectors]').textContent).toContain('Email addresses');
    expect(el(fixture, '[data-testid=rc-detectors]').textContent).toContain('brand-new');
    expect(backend.redactedCopyPreview).toHaveBeenCalledWith({ detectors: ['email'], rules: ['order-ref'], mode: 'hash' });
    expect(el(fixture, '[data-testid=rc-scanned]').textContent).toContain('Scanned 10 events, 3 would change.');
    expect(el(fixture, '[data-testid=rc-samples]').textContent).toContain('<email:1a>');
    expect(el(fixture, '[data-testid=rc-excluded]').textContent).toContain('orders.csv — CSV cannot be redacted');
    expect(el(fixture, '[data-testid=rc-notes]').textContent).toContain('video frames');
    expect(el(fixture, '[data-testid=rc-notes]').textContent).toContain("profile's own redaction rules");
    expect(createBtn(fixture).disabled).toBe(false);
  });

  it('asks again (debounced, once) when picks change', async () => {
    const previewFn = vi.fn(async (o: { detectors?: string[] }) => preview(o.detectors?.length ?? 0));
    const { fixture } = await setup({ preview: previewFn });
    previewFn.mockClear();
    box(fixture, '[data-detector=ipv4]').click();
    box(fixture, '[data-detector=brand-new]').click();
    box(fixture, '[data-mode=drop]').click();
    await settle(fixture);
    expect(previewFn).not.toHaveBeenCalled();
    await wait();
    await settle(fixture);
    expect(previewFn).toHaveBeenCalledTimes(1);
    expect(previewFn).toHaveBeenCalledWith({ detectors: ['email', 'ipv4', 'brand-new'], rules: ['order-ref'], mode: 'drop' });
    expect(el(fixture, '[data-testid=rc-scanned]').textContent).toContain('3 would change');
    box(fixture, '[data-rule=order-ref]').click();
    await wait();
    await settle(fixture);
    expect(previewFn).toHaveBeenLastCalledWith({ detectors: ['email', 'ipv4', 'brand-new'], rules: [], mode: 'drop' });
  });

  it("shows the engine's refusal and disables Create when nothing is picked", async () => {
    const previewFn = vi.fn(async (o: { detectors?: string[]; rules?: string[] }) => {
      if (!o.detectors?.length && !o.rules?.length) throw new CmdError({ code: 'invalid', message: 'Pick at least one detector or rule.', retryable: false, field: 'options' });
      return preview(1);
    });
    const { fixture } = await setup({ preview: previewFn });
    box(fixture, '[data-detector=email]').click();
    box(fixture, '[data-rule=order-ref]').click();
    await wait();
    await settle(fixture);
    expect(el(fixture, '[data-testid=rc-preview-error]').textContent).toContain('Pick at least one detector or rule.');
    expect(createBtn(fixture).disabled).toBe(true);
    box(fixture, '[data-detector=email]').click();
    await wait();
    await settle(fixture);
    expect(all(fixture, '[data-testid=rc-preview-error]')).toHaveLength(0);
    expect(createBtn(fixture).disabled).toBe(false);
  });

  it('ignores a slow answer that was overtaken by a newer one', async () => {
    let first: ((p: RedactedCopyPreview) => void) | undefined;
    const previewFn = vi.fn((o: { mode?: string }) => (o.mode === 'hash' ? new Promise<RedactedCopyPreview>((r) => (first = r)) : Promise.resolve(preview(7))));
    const { fixture } = await setup({ preview: previewFn });
    box(fixture, '[data-mode=drop]').click();
    await wait();
    await settle(fixture);
    first?.(preview(1));
    await settle(fixture);
    expect(el(fixture, '[data-testid=rc-scanned]').textContent).toContain('7 would change');
  });

  it('reports a choices failure', async () => {
    const { fixture } = await setup({ choices: vi.fn(async () => Promise.reject(new Error('engine down'))) });
    expect(el(fixture, '[data-testid=rc-load-error]').textContent).toContain('engine down');
    expect(createBtn(fixture).disabled).toBe(true);
  });

  it('creates the copy: asks for the path, sends the picks, ui and reports, then shows the result', async () => {
    const { fixture, backend, pickSavePath } = await setup();
    TestBed.inject(ProjectService).name.set('Outage');
    createBtn(fixture).click();
    await settle(fixture);
    expect(pickSavePath).toHaveBeenCalledWith('Save redacted copy', 'Outage-redacted.debriefz', [{ name: 'Debrief project', extensions: ['debriefz'] }]);
    expect(backend.exportRedactedCopy).toHaveBeenCalledWith(expect.objectContaining({ path: '/out/c.debriefz', name: 'Outage', options: { detectors: ['email'], rules: ['order-ref'], mode: 'hash' }, overwrite: true, reports: [] }));
    expect(el(fixture, '[data-testid=rc-result-summary]').textContent).toContain('2 sources and 11 events written (2 KB)');
    expect(el(fixture, '[data-testid=rc-result]').textContent).toContain('Email addresses3');
    expect(el(fixture, '[data-testid=rc-result-excluded]').textContent).toContain('orders.csv');
  });

  it('shows a busy state while writing', async () => {
    let done!: (r: RedactedExport) => void;
    const { fixture } = await setup({ export: vi.fn(() => new Promise<RedactedExport>((r) => (done = r))) });
    createBtn(fixture).click();
    await settle(fixture);
    expect(createBtn(fixture).disabled).toBe(true);
    expect(createBtn(fixture).textContent).toContain('Creating the copy');
    done(RESULT);
    await settle(fixture);
    expect(all(fixture, '[data-testid=rc-result]')).toHaveLength(1);
  });

  it('shows a failure and retries', async () => {
    const exportFn = vi.fn().mockRejectedValueOnce(new Error('disk full')).mockResolvedValue(RESULT);
    const { fixture } = await setup({ export: exportFn });
    createBtn(fixture).click();
    await settle(fixture);
    expect(el(fixture, '[data-testid=rc-create-error]').textContent).toContain('disk full');
    expect(createBtn(fixture).textContent).toContain('Try again');
    createBtn(fixture).click();
    await settle(fixture);
    expect(exportFn).toHaveBeenCalledTimes(2);
    expect(all(fixture, '[data-testid=rc-result]')).toHaveLength(1);
  });

  it('does nothing when the save dialog is cancelled', async () => {
    const { fixture, backend } = await setup({}, null);
    createBtn(fixture).click();
    await settle(fixture);
    expect(backend.exportRedactedCopy).not.toHaveBeenCalled();
    expect(createBtn(fixture).disabled).toBe(false);
  });

  it('keeps the dialog open without a result when the user declines to replace an existing file', async () => {
    const exists = new Error('/out/c.debriefz already exists; confirm to replace it');
    const { fixture } = await setup({ export: vi.fn().mockRejectedValue(exists) });
    vi.spyOn(TestBed.inject(OverwriteGuard), 'confirm').mockResolvedValue(false);
    createBtn(fixture).click();
    await settle(fixture);
    expect(all(fixture, '[data-testid=rc-result]')).toHaveLength(0);
    expect(all(fixture, '[data-testid=rc-create-error]')).toHaveLength(0);
  });

  it('closes with Done and Cancel', async () => {
    const { fixture, ref } = await setup();
    el(fixture, '[data-testid=rc-cancel]').click();
    expect(ref.close).toHaveBeenCalledWith();
    createBtn(fixture).click();
    await settle(fixture);
    el(fixture, '[data-testid=rc-done]').click();
    expect(ref.close).toHaveBeenCalledWith(true);
  });
});
