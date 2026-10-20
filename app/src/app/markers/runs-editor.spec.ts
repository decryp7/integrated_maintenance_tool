import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend, typeInto } from '../../testing/helpers';
import { MockBackend } from '../core/mock/mock-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { parseProfile } from '../profile/toml';
import { RunsBackend } from '../timeline/runs-backend';
import { RunsEditor } from './runs-editor';

/** A profile save re-imports as a job: waits for it to finish and for the view to follow. */
async function applied(fixture?: Parameters<typeof settle>[0]): Promise<void> {
  // The import's progress is polled on a timer: advance the fake clock instead of waiting for wall time,
  // which a loaded CI machine stretches past any fixed limit.
  await vi.waitFor(
    async () => {
      await vi.advanceTimersByTimeAsync(250);
      expect(TestBed.inject(WorkspaceStore).busy()).toBeNull();
    },
    { timeout: 20_000, interval: 20 },
  );
  await settle(fixture);
}

async function mount(backend: MockBackend = tinyBackend()) {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
  await TestBed.inject(WorkspaceStore).refreshSummary();
  const fixture = TestBed.createComponent(RunsEditor);
  await settle(fixture, 8);
  return { fixture, backend };
}

/** Lets the debounced preview run and its answer render. */
async function previewed(fixture: Parameters<typeof settle>[0]) {
  await vi.advanceTimersByTimeAsync(400);
  await settle(fixture, 8);
}

const model = async (b: MockBackend) => parseProfile((await b.getProfile()).toml);

describe('RunsEditor', () => {
  afterEach(() => vi.useRealTimers());

  it('lists the run rules with their counts', async () => {
    const { fixture } = await mount();
    const row = el(fixture, '.rules li[data-kind="run"]');
    expect(row.textContent).toContain('Running');
    expect(row.textContent).toContain('start → stop, per lane');
    expect(row.textContent).toContain('3 runs, 0 anomalies');
  });

  it('creates a rule from the service preset with a live preview and saves it', async () => {
    const { fixture, backend } = await mount();
    el<HTMLButtonElement>(fixture, '[data-testid="runs-new"]').click();
    await previewed(fixture);
    expect(el<HTMLInputElement>(fixture, 'input[name="label"]').value).toBe('Service running');
    expect(el(fixture, '[data-testid="runs-preview"]').textContent).toContain('3 runs on 3 lanes');
    expect(all(fixture, '.mini tbody tr').length).toBe(3);
    const save = el<HTMLButtonElement>(fixture, '[data-testid="runs-save"]');
    expect(save.disabled).toBe(false);
    save.click();
    await settle(fixture, 12);
    await applied(fixture);
    const m = await model(backend);
    expect((m.span ?? []).map((s) => s.kind)).toEqual(['run', 'service-running']);
    expect(fixture.nativeElement.querySelector('[data-testid="runs-form"]')).toBeNull();
  });

  it('creates marker rules for text patterns and pairs per a field', async () => {
    const { fixture, backend } = await mount();
    el<HTMLButtonElement>(fixture, '[data-testid="runs-new"]').click();
    await settle(fixture, 4);
    const radios = all<HTMLInputElement>(fixture, 'input[name="preset"]');
    radios[1]!.click();
    await previewed(fixture);
    expect(el<HTMLSelectElement>(fixture, 'select[name="per"]').value).toBe('ProcessId');
    expect(el<HTMLInputElement>(fixture, 'input[name="start-pattern"]').value).toContain('started');
    // Type an invalid pattern: the engine's message shows next to the field and saving is blocked.
    typeInto(el<HTMLInputElement>(fixture, 'input[name="start-pattern"]'), '(');
    await previewed(fixture);
    expect(el(fixture, '#runs-err-start').textContent).toContain('not a valid regex');
    expect(el<HTMLInputElement>(fixture, 'input[name="start-pattern"]').getAttribute('aria-describedby')).toBeNull();
    expect(el<HTMLSelectElement>(fixture, 'select[name="start-mode"]').getAttribute('aria-describedby')).toBe('runs-err-start');
    expect(el<HTMLButtonElement>(fixture, '[data-testid="runs-save"]').disabled).toBe(true);
    typeInto(el<HTMLInputElement>(fixture, 'input[name="start-pattern"]'), 'Application starting');
    typeInto(el<HTMLInputElement>(fixture, 'input[name="stop-pattern"]'), 'shutting down');
    typeInto(el<HTMLInputElement>(fixture, 'input[name="maxDuration"]'), '1h');
    await previewed(fixture);
    el<HTMLButtonElement>(fixture, '[data-testid="runs-save"]').click();
    await settle(fixture, 12);
    await applied(fixture);
    const m = await model(backend);
    const span = (m.span ?? []).find((s) => s.kind === 'process');
    expect(span).toMatchObject({ start: 'process-start', stop: 'process-stop', per: 'field:ProcessId', max_duration: '1h' });
    expect((m.marker ?? []).map((x) => x.kind)).toEqual(expect.arrayContaining(['process-start', 'process-stop']));
  });

  it('switches to markers or patterns per side, edits, switches off and deletes rules', async () => {
    const { fixture, backend } = await mount();
    el<HTMLButtonElement>(fixture, '[aria-label="Edit Running"]').click();
    await previewed(fixture);
    expect(text(fixture)).toContain('Edit run rule');
    expect(text(fixture)).toContain('replaces the existing run rule');
    typeInto(el<HTMLSelectElement>(fixture, 'select[name="stop-mode"]'), 'p:', 'change');
    await settle(fixture, 4);
    expect(fixture.nativeElement.querySelector('input[name="stop-pattern"]')).not.toBeNull();
    typeInto(el<HTMLSelectElement>(fixture, 'select[name="stop-mode"]'), 'm:fatal', 'change');
    el<HTMLInputElement>(fixture, 'input[name="restart"]').click();
    typeInto(el<HTMLInputElement>(fixture, 'input[name="color"]'), 'red');
    await previewed(fixture);
    expect(el(fixture, '#runs-err-color').textContent).toContain('#RGB');
    typeInto(el<HTMLInputElement>(fixture, 'input[name="color"]'), '#3987e5');
    typeInto(el<HTMLInputElement>(fixture, 'input[name="label"]'), 'Up');
    await previewed(fixture);
    el<HTMLButtonElement>(fixture, '[data-testid="runs-save"]').click();
    await settle(fixture, 12);
    await applied(fixture);
    let m = await model(backend);
    expect(m.span).toEqual([{ kind: 'run', label: 'Up', start: 'start', stop: 'fatal', per: 'instance', color: '#3987e5', restart_on_start: false }]);

    el<HTMLInputElement>(fixture, '.rules li[data-kind="run"] input[type="checkbox"]').click();
    await settle(fixture, 12);
    await applied(fixture);
    m = await model(backend);
    expect(m.span?.[0]?.enabled).toBe(false);
    el<HTMLButtonElement>(fixture, '[aria-label="Delete Up"]').click();
    await settle(fixture, 2);
    buttonByText(fixture, 'Keep').click();
    await settle(fixture, 2);
    el<HTMLButtonElement>(fixture, '[aria-label="Delete Up"]').click();
    await settle(fixture, 2);
    buttonByText(fixture, 'Confirm delete').click();
    await settle(fixture, 12);
    await applied(fixture);
    m = await model(backend);
    expect(m.span).toEqual([]);
  });

  it('shows a failed preview and a failed save, and cancels', async () => {
    const { fixture } = await mount();
    const runs = TestBed.inject(RunsBackend);
    vi.spyOn(runs, 'previewSpans').mockRejectedValueOnce('engine down');
    el<HTMLButtonElement>(fixture, '[data-testid="runs-new"]').click();
    await previewed(fixture);
    expect(el(fixture, '[data-testid="runs-preview"]').textContent).toContain('engine down');
    typeInto(el<HTMLInputElement>(fixture, 'input[name="label"]'), 'Again');
    await previewed(fixture);
    expect(el(fixture, '[data-testid="runs-preview"]').textContent).toContain('runs on');
    buttonByText(fixture, 'Cancel').click();
    await settle(fixture, 4);
    expect(fixture.nativeElement.querySelector('[data-testid="runs-form"]')).toBeNull();
  });
});
