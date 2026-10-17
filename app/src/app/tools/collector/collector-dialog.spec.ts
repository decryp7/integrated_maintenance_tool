import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, settle, text, typeInto } from '../../../testing/helpers';
import { Platform } from '../../core/platform';
import { WorkspaceStore } from '../../core/workspace.store';
import { DIALOG_REF, DialogService } from '../../ui/dialog.service';
import { CollectBackend } from '../../core/tools-backend';
import { provideToolsTesting } from '../mock/test-support';
import { CollectorDialog, CollectorDialogService } from './collector-dialog';

async function setup(pick: string[] = ['/picked']) {
  const close = vi.fn();
  TestBed.configureTestingModule({
    providers: [
      ...provideToolsTesting(),
      { provide: DIALOG_REF, useValue: { close, closed: Promise.resolve(undefined) } },
      { provide: Platform, useValue: { pickPaths: async () => pick, onFilesDropped: async () => () => undefined } },
    ],
  });
  const fixture = TestBed.createComponent(CollectorDialog);
  await settle(fixture);
  return { fixture, close, backend: TestBed.inject(CollectBackend), workspace: TestBed.inject(WorkspaceStore) };
}

describe('CollectorDialogService', () => {
  it('opens the dialog', () => {
    TestBed.configureTestingModule({ providers: provideToolsTesting() });
    TestBed.inject(CollectorDialogService).open();
    expect(TestBed.inject(DialogService).stack()[0].options.title).toBe('Collect diagnostics bundle');
  });
});

describe('CollectorDialog', () => {
  it('edits the plan through the form and runs the collection with the resulting JSON', async () => {
    const { fixture, backend } = await setup();
    const run = vi.spyOn(backend, 'collectDiagnostics');
    typeInto(el(fixture, 'input[aria-label="Bundle name"]'), 'repro');
    buttonByText(fixture, /Add log folder/).click();
    await settle(fixture);
    typeInto(el(fixture, 'input[aria-label="Log folder 1"]'), 'C:\\Logs');
    typeInto(el(fixture, 'input[aria-label="Include globs 1"]'), '*.log, *.txt, ,');
    typeInto(el(fixture, 'input[aria-label="Max age 1"]'), '3');
    typeInto(el(fixture, 'input[aria-label="Max size 1"]'), '50');
    buttonByText(fixture, /Add log folder/).click();
    await settle(fixture);
    el<HTMLButtonElement>(fixture, 'button[aria-label="Remove log folder 2"]').click();

    const checks = all<HTMLInputElement>(fixture, '.checks input');
    checks.forEach((c) => c.click()); // flips environment, msinfo, counters, macOS, linux, built-in and IP redaction
    await settle(fixture);
    typeInto(el(fixture, 'textarea[aria-label="Event logs"]'), 'System\n\nSecurity');
    typeInto(el(fixture, 'textarea[aria-label="Registry keys"]'), 'HKLM\\SOFTWARE\\X');
    typeInto(el(fixture, 'textarea[aria-label="Counter paths"]'), '\\Memory\\Pages/sec');
    const numbers = all<HTMLInputElement>(fixture, 'fieldset input[type=number]:not([aria-label])');
    numbers.forEach((n, i) => typeInto(n, String(100 + i)));

    buttonByText(fixture, /Add custom pattern/).click();
    await settle(fixture);
    typeInto(el(fixture, 'input[aria-label="Pattern name 1"]'), 'order');
    typeInto(el(fixture, 'input[aria-label="Pattern regex 1"]'), 'ORD-\\d+');
    typeInto(el(fixture, 'input[aria-label="Pattern replacement 1"]'), '[ORDER]');
    buttonByText(fixture, /Add custom pattern/).click();
    await settle(fixture);
    el<HTMLButtonElement>(fixture, 'button[aria-label="Remove pattern 2"]').click();

    typeInto(el(fixture, 'input[aria-label="Output folder"]'), '/out');
    buttonByText(fixture, 'Collect').click();
    await settle(fixture);
    const plan = JSON.parse(run.mock.calls[0][0]);
    expect(plan).toMatchObject({
      name: 'repro',
      logs: [{ path: 'C:\\Logs', include: ['*.log', '*.txt'], maxAgeDays: 3, maxTotalMb: 50 }],
      environment: false,
      msinfo: true,
      counters: true,
      macosUnifiedLog: true,
      linuxJournal: true,
      eventLogs: ['System', 'Security'],
      registry: ['HKLM\\SOFTWARE\\X'],
      counterPaths: ['\\Memory\\Pages/sec'],
      redact: { builtIn: false, ipAddresses: false, patterns: [{ name: 'order', regex: 'ORD-\\d+', replace: '[ORDER]' }] },
    });
    expect(plan.eventLogDays).toBe(100);
    expect(run.mock.calls[0][1]).toBe('/out');
  });

  it('shows the manifest summary and opens the bundle', async () => {
    const { fixture, close, workspace } = await setup();
    typeInto(el(fixture, 'input[aria-label="Output folder"]'), '/out');
    buttonByText(fixture, 'Collect').click();
    await settle(fixture);
    const result = el(fixture, '[data-testid=collect-result]').textContent ?? '';
    expect(result).toContain('/out/imt-diagnostics.imtz');
    expect(result).toContain('2 items');
    expect(result).toContain('3 values masked');
    expect(result).toContain('(email 2, token 1)');
    expect(result).toContain('1 skipped');
    expect(result).toContain('msinfo32: disabled in the plan');
    expect(result).toContain('0 errors');
    expect(buttonByText(fixture, 'Collect again')).toBeTruthy();
    buttonByText(fixture, 'Open bundle').click();
    await settle(fixture);
    expect(workspace.summary()?.sources.some((s) => s.path.endsWith('.imtz'))).toBe(true);
    expect(close).toHaveBeenCalled();
  });

  it('shows errors from the manifest and keeps the dialog open when the bundle fails to open', async () => {
    const { fixture, close, backend, workspace } = await setup();
    vi.spyOn(backend, 'collectDiagnostics').mockResolvedValue({
      bundle: '/b.imtz',
      manifest: { name: 'n', host: 'h', platform: 'p', createdAt: 't', items: [], skipped: [], redaction: {}, errors: ['wevtutil failed', 'x'], totalBytes: 0, durationMs: 1 },
    });
    typeInto(el(fixture, 'input[aria-label="Output folder"]'), '/out');
    buttonByText(fixture, 'Collect').click();
    await settle(fixture);
    expect(text(fixture)).toContain('wevtutil failed');
    expect(text(fixture)).toContain('0 values masked');
    expect(text(fixture)).toContain('0 items');
    vi.spyOn(workspace, 'openPaths').mockImplementation(async () => workspace.setError('bad'));
    buttonByText(fixture, 'Open bundle').click();
    await settle(fixture);
    expect(close).not.toHaveBeenCalled();
  });

  it('needs an output folder and reports engine errors', async () => {
    const { fixture, backend } = await setup();
    buttonByText(fixture, 'Collect').click();
    await settle(fixture);
    expect(text(fixture)).toContain('Choose an output folder');
    vi.spyOn(backend, 'collectDiagnostics').mockRejectedValue('unknown key "bogus"');
    typeInto(el(fixture, 'input[aria-label="Output folder"]'), '/out');
    buttonByText(fixture, 'Collect').click();
    await settle(fixture);
    expect(text(fixture)).toContain('unknown key "bogus"');
    el<HTMLButtonElement>(fixture, '.err .btn.ghost').click();
    await settle(fixture);
    expect(text(fixture)).not.toContain('unknown key');
  });

  it('shows progress while collecting', async () => {
    const { fixture, backend } = await setup();
    let finish!: (v: Awaited<ReturnType<typeof backend.collectDiagnostics>>) => void;
    vi.spyOn(backend, 'collectDiagnostics').mockReturnValue(new Promise((r) => (finish = r)));
    typeInto(el(fixture, 'input[aria-label="Output folder"]'), '/out');
    buttonByText(fixture, 'Collect').click();
    await settle(fixture);
    expect(el(fixture, '[data-testid=collect-progress]')).toBeTruthy();
    expect(buttonByText(fixture, 'Collect').disabled).toBe(true);
    expect(el<HTMLButtonElement>(fixture, '.bar button').disabled).toBe(true);
    finish({ bundle: '/b', manifest: { name: 'n', host: '', platform: '', createdAt: '', items: [], skipped: [], redaction: {}, errors: [], totalBytes: 0, durationMs: 0 } });
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('[data-testid=collect-progress]')).toBeNull();
  });

  it('edits the plan as JSON and brings the edits back to the form', async () => {
    const { fixture, backend } = await setup();
    const run = vi.spyOn(backend, 'collectDiagnostics');
    buttonByText(fixture, 'Edit as JSON').click();
    await settle(fixture);
    const area = el<HTMLTextAreaElement>(fixture, 'textarea[aria-label="Plan JSON"]');
    expect(JSON.parse(area.value).name).toBe('imt-diagnostics');
    typeInto(area, '{ broken');
    await settle(fixture);
    expect(text(fixture)).toContain('Not valid JSON');
    expect(buttonByText(fixture, 'Collect').disabled).toBe(true);
    buttonByText(fixture, 'Edit as JSON').click(); // refuses to leave while invalid
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('textarea[aria-label="Plan JSON"]')).not.toBeNull();
    typeInto(area, '{ "name": "from-json", "msinfo": true }');
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('.err')).toBeNull();
    typeInto(el(fixture, 'input[aria-label="Output folder"]'), '/o');
    buttonByText(fixture, 'Collect').click();
    await settle(fixture);
    expect(JSON.parse(run.mock.calls[0][0])).toMatchObject({ name: 'from-json', msinfo: true });
    buttonByText(fixture, 'Edit as JSON').click();
    await settle(fixture);
    expect(el<HTMLInputElement>(fixture, 'input[aria-label="Bundle name"]').value).toBe('from-json');
  });

  it('browses for the output folder and handles a failing picker', async () => {
    const { fixture } = await setup(['/picked']);
    buttonByText(fixture, /Browse/).click();
    await settle(fixture);
    expect(el<HTMLInputElement>(fixture, 'input[aria-label="Output folder"]').value).toBe('/picked');
    TestBed.resetTestingModule();
    const cancelled = await setup([]);
    buttonByText(cancelled.fixture, /Browse/).click();
    await settle(cancelled.fixture);
    expect(el<HTMLInputElement>(cancelled.fixture, 'input[aria-label="Output folder"]').value).toBe('');
    TestBed.resetTestingModule();
    const broken = await setup();
    TestBed.inject(Platform).pickPaths = async () => Promise.reject(new Error('no dialog'));
    buttonByText(broken.fixture, /Browse/).click();
    await settle(broken.fixture);
    expect(text(broken.fixture)).toContain('no dialog');
  });

  it('survives a default plan that cannot be loaded or parsed', async () => {
    TestBed.configureTestingModule({
      providers: [...provideToolsTesting(), { provide: DIALOG_REF, useValue: { close: () => undefined } }],
    });
    const backend = TestBed.inject(CollectBackend);
    vi.spyOn(backend, 'collectDefaultPlan').mockRejectedValue(new Error('offline'));
    const a = TestBed.createComponent(CollectorDialog);
    await settle(a);
    expect(text(a)).toContain('Could not load the default plan: offline');
    vi.spyOn(backend, 'collectDefaultPlan').mockResolvedValue('[1]');
    const b = TestBed.createComponent(CollectorDialog);
    await settle(b);
    expect(text(b)).toContain('must be a JSON object');
  });

  it('closes without collecting', async () => {
    const { fixture, close } = await setup();
    buttonByText(fixture, 'Close').click();
    expect(close).toHaveBeenCalled();
  });
});
