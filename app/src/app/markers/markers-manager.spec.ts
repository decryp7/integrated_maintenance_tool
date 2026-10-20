import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend } from '../../testing/helpers';
import { MarkDialog } from '../context/mark-dialog';
import { MockBackend } from '../core/mock/mock-backend';
import { parseProfile } from '../profile/toml';
import { DialogService } from '../ui/dialog.service';
import { MarkersManager } from './markers-manager';
import { MarkerRuleX } from './markers.types';
import { at } from '../../testing/must';
import { WorkspaceStore } from '../core/workspace.store';
import { mockJobs } from '../jobs/mock-jobs';

/** A profile save re-imports as a job: waits for it to finish and for the view to follow. */
async function applied(fixture?: Parameters<typeof settle>[0]): Promise<void> {
  await vi.waitFor(() => expect(TestBed.inject(WorkspaceStore).busy()).toBeNull(), { timeout: 5000 });
  await settle(fixture);
}

async function mount(backend: MockBackend = tinyBackend()) {
  TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
  const fixture = TestBed.createComponent(MarkersManager);
  await settle(fixture, 8);
  await applied(fixture);
  return { fixture, backend };
}
const kinds = (f: Parameters<typeof all>[0]) => all(f, 'tbody tr').map((r) => r.getAttribute('data-kind'));
const btn = (f: Parameters<typeof all>[0], label: string) => el<HTMLButtonElement>(f, `[aria-label="${label}"]`);
const model = async (b: MockBackend) => parseProfile((await b.getProfile()).toml);

describe('MarkersManager', () => {
  it('lists the rules in priority order with look, meaning, condition and live counts', async () => {
    const { fixture } = await mount();
    expect(kinds(fixture)).toEqual(['fatal', 'start', 'stop', 'exception', 'error', 'warning']);
    expect(text(fixture)).toContain('level is "Fatal"');
    const row = el(fixture, 'tr[data-kind="error"]');
    expect(row.querySelector('.num')!.textContent!.trim()).toBe('1');
    expect(row.querySelector('imt-marker-glyph')).not.toBeNull();
  });

  it('reorders with the move buttons and saves the new priority', async () => {
    const { fixture, backend } = await mount();
    expect(btn(fixture, 'Move up Fatal').disabled).toBe(true);
    expect(btn(fixture, 'Move down Warning').disabled).toBe(true);
    btn(fixture, 'Move down Fatal').click();
    await settle(fixture, 8);
    await applied(fixture);
    expect(kinds(fixture).slice(0, 2)).toEqual(['start', 'fatal']);
    expect(((await model(backend)).marker as MarkerRuleX[]).slice(0, 2).map((m) => m.kind)).toEqual(['start', 'fatal']);
  });

  it('reorders on drop', async () => {
    const { fixture, backend } = await mount();
    const drop = (fixture.componentInstance as unknown as { dropped(e: unknown): Promise<void> }).dropped({ previousIndex: 5, currentIndex: 0 });
    await drop;
    await settle(fixture, 6);
    expect(kinds(fixture)[0]).toBe('warning');
    expect(at((await model(backend)).marker as MarkerRuleX[], 0).kind).toBe('warning');
  });

  it('disables a rule into imt_disabled_markers and re-enables it at the same place', async () => {
    const { fixture, backend } = await mount();
    btn(fixture, 'Disable Start').click();
    await settle(fixture, 8);
    await applied(fixture);
    let m = await model(backend);
    expect((m.marker as MarkerRuleX[]).map((r) => r.kind)).not.toContain('start');
    expect((m['imt_disabled_markers'] as MarkerRuleX[])[0]).toMatchObject({ kind: 'start', imt_index: 1 });
    expect(kinds(fixture)[1]).toBe('start');
    expect(el(fixture, 'tr[data-kind="start"]').classList).toContain('off');
    expect(el(fixture, 'tr[data-kind="start"] .num').textContent!.trim()).toBe('–');
    btn(fixture, 'Enable Start').click();
    await settle(fixture, 8);
    await applied(fixture);
    m = await model(backend);
    expect((m.marker as MarkerRuleX[]).map((r) => r.kind).slice(0, 3)).toEqual(['fatal', 'start', 'stop']);
    expect(m).not.toHaveProperty('imt_disabled_markers');
  });

  it('duplicates a rule without its badge', async () => {
    const { fixture, backend } = await mount();
    btn(fixture, 'Duplicate Error').click();
    await settle(fixture, 8);
    await applied(fixture);
    const list = (await model(backend)).marker as MarkerRuleX[];
    expect(list.map((r) => r.kind)).toContain('error-copy');
    expect(list.find((r) => r.kind === 'error-copy')!.label).toBe('Error (copy)');
  });

  it('asks before deleting, and deleting removes the rule', async () => {
    const { fixture, backend } = await mount();
    btn(fixture, 'Delete Warning').click();
    await settle(fixture);
    buttonByText(fixture, 'Keep').click();
    await settle(fixture);
    expect(kinds(fixture)).toContain('warning');
    btn(fixture, 'Delete Warning').click();
    await settle(fixture);
    buttonByText(fixture, 'Confirm delete').click();
    await settle(fixture, 8);
    await applied(fixture);
    expect(kinds(fixture)).not.toContain('warning');
    expect(((await model(backend)).marker as MarkerRuleX[]).map((r) => r.kind)).not.toContain('warning');
  });

  it('opens the shared editor to edit or add', async () => {
    const { fixture } = await mount();
    const dialogs = TestBed.inject(DialogService);
    btn(fixture, 'Edit Error').click();
    buttonByText(fixture, /New marker/).click();
    const stack = dialogs.stack();
    const edit = at(stack, 0);
    const add = at(stack, 1);
    expect(edit.component).toBe(MarkDialog);
    expect(edit.options.title).toBe('Edit marker "Error"');
    expect(edit.options.inputs?.['rule']).toMatchObject({ kind: 'error' });
    expect(add.options.inputs?.['initial']).toEqual([{ field: '@message', op: 'matches', value: '' }]);
  });

  it('shows failures with retry and rolls the list back', async () => {
    const { fixture, backend } = await mount();
    vi.spyOn(backend, 'validateProfile').mockResolvedValueOnce({ ok: false, error: 'nope', line: 7 });
    btn(fixture, 'Move down Fatal').click();
    await settle(fixture, 8);
    await applied(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('nope (profile line 7)');
    expect(kinds(fixture)[0]).toBe('fatal');
    vi.spyOn(backend, 'getProfile').mockRejectedValueOnce(new Error('unreadable'));
    (fixture.componentInstance as unknown as { reload(): void }).reload();
    await settle(fixture, 4);
    expect(el(fixture, '[role=alert]').textContent).toContain('Could not load the markers: unreadable');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture, 4);
    expect(fixture.nativeElement.querySelector('[role=alert]')).toBeNull();
  });

  it('tolerates a failing count and an empty list', async () => {
    const backend = tinyBackend();
    vi.spyOn(backend, 'queryPage').mockRejectedValue(new Error('x'));
    const { fixture } = await mount(backend);
    expect(el(fixture, 'tr[data-kind="error"] .num').textContent!.trim()).toBe('…');
    TestBed.resetTestingModule();
    const empty = tinyBackend();
    await mockJobs().wait(await empty.saveProfile('name = "x"\n'));
    const e = await mount(empty);
    expect(text(e.fixture)).toContain('No marker rules');
  });
});
