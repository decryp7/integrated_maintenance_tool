import { ComponentFixture, TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { all, buttonByText, el, settle, text, typeInto } from '../../testing/helpers';
import { CmdError } from '../core/cmd-error';
import { DumpView } from './dump-view';
import { DumpsExplainer } from './dumps-shared';
import { DumpsStore } from './dumps.store';
import { MockDumpsBackend } from './mock/mock-dumps-backend';
import { setupDumps } from './testing/dumps-test-support';

async function mount(open = true, backend?: MockDumpsBackend) {
  const t = setupDumps({ backend });
  const explain = vi.fn(async () => true);
  TestBed.overrideProvider(DumpsExplainer, { useValue: { explain } });
  if (open) await TestBed.inject(DumpsStore).pickAndOpen('dump');
  const fixture: ComponentFixture<DumpView> = TestBed.createComponent(DumpView);
  fixture.detectChanges();
  await settle(fixture, 6);
  return { fixture, t, explain, view: fixture.componentInstance, store: TestBed.inject(DumpsStore) };
}

const tab = (f: ComponentFixture<DumpView>, id: string) => el<HTMLButtonElement>(f, `[data-testid="dump-tab-${id}"]`);

describe('DumpView', () => {
  it('offers to open a dump when none is open', async () => {
    const { fixture, t } = await mount(false);
    expect(text(fixture)).toContain('Open a crash or memory dump');
    buttonByText(fixture, 'Open dump…').click();
    await settle(fixture, 6);
    expect(text(fixture)).toContain('CLR_EXCEPTION');
    expect(t.picker.dumps).toHaveLength(1);
  });

  it('opens on the findings, under the exception card', async () => {
    const { fixture } = await mount();
    expect(tab(fixture, 'findings').getAttribute('aria-selected')).toBe('true');
    const card = el(fixture, '[data-testid="dump-exception"]');
    expect(card.textContent).toContain('CLR_EXCEPTION');
    expect(card.textContent).toContain('0xE0434352');
    expect(card.textContent).toContain('KERNELBASE.dll!RaiseException+0x6c');
    const findings = all(fixture, 'li.finding');
    expect(findings.map((f) => f.getAttribute('data-kind'))).toEqual(['crashCause', 'culprit', 'lockWait', 'dotnet']);
    expect(text(fixture)).toContain('OrderService.exe');
    expect(text(fixture)).toContain('x64');
    expect(text(fixture)).toContain('.NET 8.0.1.0');
  });

  it('shows the culprit in the faulting thread from its finding', async () => {
    const { fixture, view } = await mount();
    el<HTMLButtonElement>(fixture, 'button[aria-label="Show the evidence of Likely culprit: Shop.Orders.dll!OrderCache::Add+0x4a"]').click();
    await settle(fixture, 8);
    expect(view.tab()).toBe('threads');
    expect(view.selectedThread()).toBe(4121);
    const rows = all(fixture, '[data-testid="dump-stack"] tbody tr');
    expect(rows).toHaveLength(6);
    const culprit = rows.find((r) => r.hasAttribute('data-culprit'))!;
    expect(culprit.textContent).toContain('likely culprit');
    expect(culprit.classList.contains('sel')).toBe(true);
    expect(rows[0]!.classList.contains('framework')).toBe(true);
    // The lock finding shows its first thread.
    tab(fixture, 'findings').click();
    await settle(fixture, 6);
    el<HTMLButtonElement>(fixture, 'button[aria-label^="Show the evidence of 2 thread(s) wait"]').click();
    await settle(fixture, 6);
    expect(view.selectedThread()).toBe(4122);
  });

  it('lists threads in a listbox, filters them and shows the chosen stack', async () => {
    const { fixture, view } = await mount();
    tab(fixture, 'threads').click();
    await settle(fixture, 8);
    const options = all(fixture, '[data-testid="dump-threads"] [role="option"]');
    expect(options.map((o) => o.getAttribute('data-state'))).toEqual(['faulted', 'lockWait', 'lockWait', 'waiting']);
    expect(view.selectedThread()).toBe(4121);
    options[3]!.click();
    await settle(fixture, 6);
    expect(view.selectedThread()).toBe(4124);
    expect(el(fixture, '[data-testid="dump-stack"]').textContent).toContain('Timer::Run');
    typeInto(el<HTMLInputElement>(fixture, 'input[type="search"]'), 'ledger');
    await settle(fixture, 6);
    expect(all(fixture, '[data-testid="dump-threads"] [role="option"]')).toHaveLength(2);
    view.selectThread('not a number');
    expect(view.selectedThread()).toBe(4124);
    // The exception card's thread link.
    buttonByText(fixture, 'thread 4121').click();
    await settle(fixture, 6);
    expect(view.selectedThread()).toBe(4121);
  });

  it('reads the dump again when its downloaded symbols arrived, and only for the dump it shows', async () => {
    const { fixture, t, store } = await mount();
    const backend = t.backend as MockDumpsBackend;
    const overview = vi.spyOn(backend, 'overview');
    const findings = vi.spyOn(backend, 'findings');
    const id = store.dump()!.id;
    store.noteSymbolsChanged(id + 100);
    await settle(fixture, 4);
    expect(overview).not.toHaveBeenCalled();
    store.noteSymbolsChanged(id);
    await settle(fixture, 4);
    expect(overview).toHaveBeenCalledTimes(1);
    expect(findings).toHaveBeenCalledTimes(1);
    // On the Modules tab the table is read again too.
    tab(fixture, 'modules').click();
    await settle(fixture, 6);
    const modules = vi.spyOn(backend, 'modules');
    store.noteSymbolsChanged(id);
    await settle(fixture, 4);
    expect(modules).toHaveBeenCalledTimes(1);
  });

  it('lists modules and details', async () => {
    const { fixture } = await mount();
    tab(fixture, 'modules').click();
    await settle(fixture, 6);
    expect(all(fixture, '[data-testid="dump-modules"] tbody tr')).toHaveLength(6);
    // Each module says what its symbols are, with the next step on hover.
    const cells = all(fixture, '[data-testid="module-symbols"]');
    expect(cells.map((c) => c.textContent?.trim())).toEqual(['Loaded', 'Loaded', 'Missing (downloads off)', 'Not on the servers', 'Loaded', 'Loaded']);
    expect(cells[2]?.getAttribute('title')).toContain('Settings > Symbols');
    expect(cells[0]?.getAttribute('title')).toBe('');
    typeInto(el<HTMLInputElement>(fixture, 'input[type="search"]'), 'shop.');
    await settle(fixture, 6);
    expect(all(fixture, '[data-testid="dump-modules"] tbody tr')).toHaveLength(2);
    tab(fixture, 'details').click();
    await settle(fixture, 4);
    const t = text(fixture);
    expect(t).toContain('Handles (412)');
    expect(t).toContain('Legacy.dll');
    expect(t).toContain('14 of 14 frames named');
    expect(el(fixture, '[data-testid="symbol-problems"]').textContent).toContain('downloads are off');
  });

  it('explains with AI and shows a failure inline', async () => {
    const { fixture, explain, store } = await mount();
    el<HTMLButtonElement>(fixture, '[data-testid="dumps-explain"]').click();
    await settle(fixture, 4);
    expect(explain).toHaveBeenCalledWith([store.dump()!.id]);
    explain.mockRejectedValueOnce(new CmdError({ code: 'invalid', message: 'nothing to explain', retryable: false }));
    el<HTMLButtonElement>(fixture, '[data-testid="dumps-explain"]').click();
    await settle(fixture, 4);
    expect(text(fixture)).toContain('nothing to explain');
  });

  it('switches between dumps and closes them', async () => {
    const { fixture, store } = await mount();
    await store.openPaths(['/samples/dumps/Billing.exe.900.hang.dmp']);
    await settle(fixture, 6);
    const pick = el<HTMLSelectElement>(fixture, '[data-testid="dump-pick"]');
    expect(pick.options).toHaveLength(2);
    pick.value = String(store.dumps()[0]!.id);
    pick.dispatchEvent(new Event('change'));
    await settle(fixture, 6);
    expect(store.activeDump()).toBe(store.dumps()[0]!.id);
    buttonByText(fixture, 'Close').click();
    await settle(fixture, 6);
    expect(store.dumps()).toHaveLength(1);
    buttonByText(fixture, 'Close').click();
    await settle(fixture, 6);
    expect(text(fixture)).toContain('Open a crash or memory dump');
  });

  it('shows a failed overview with Retry', async () => {
    const backend = new MockDumpsBackend();
    let fail = true;
    const real = backend.overview.bind(backend);
    backend.overview = (id) => (fail ? Promise.reject(new CmdError({ code: 'io', message: 'cannot read the dump', retryable: true })) : real(id));
    const { fixture } = await mount(true, backend);
    expect(text(fixture)).toContain('cannot read the dump');
    fail = false;
    buttonByText(fixture, 'Retry').click();
    await settle(fixture, 6);
    expect(el(fixture, '[data-testid="dump-exception"]').textContent).toContain('CLR_EXCEPTION');
  });
});
