import { Type } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { all, buttonByText, el, settle, text } from '../../../testing/helpers';
import { DIALOG_REF } from '../../ui/dialog.service';
import { setupProfiling } from '../testing/profiling-test-support';
import { DotnetBackend, ToolDotnetBackend } from './dotnet-backend';
import { DotnetCaptureStore, DotnetOpener } from './dotnet-capture.store';
import { DotnetDialog, KIND_CHOICES } from './dotnet-dialog';
import { MOCK_PROFILE_PATH, MockDotnetBackend } from './mock-dotnet-backend';
import { ToolCatalogBackend } from '../../core/tools-backend';
import { JobsBackend } from '../../jobs/jobs-backend';
import { provideToolsTesting } from '../../tools/mock/test-support';

class FakeOpener {
  opened: string[] = [];
  async open(path: string) {
    this.opened.push(path);
  }
}

async function mount<T>(component: Type<T>, inputs: Record<string, unknown> = {}) {
  const backend = new MockDotnetBackend();
  const opener = new FakeOpener();
  const closed: unknown[] = [];
  setupProfiling();
  TestBed.overrideProvider(DotnetBackend, { useValue: backend });
  TestBed.overrideProvider(DotnetOpener, { useValue: opener });
  TestBed.configureTestingModule({ providers: [...provideToolsTesting(), { provide: DIALOG_REF, useValue: { close: (r?: unknown) => closed.push(r ?? 'closed'), closed: Promise.resolve(undefined) } }] });
  const fixture: ComponentFixture<T> = TestBed.createComponent(component);
  for (const [k, v] of Object.entries(inputs)) fixture.componentRef.setInput(k, v);
  fixture.detectChanges();
  await settle(fixture, 6);
  return { fixture, backend, opener, closed };
}

describe('DotnetDialog', () => {
  beforeEach(() => vi.useFakeTimers({ shouldAdvanceTime: true }));
  afterEach(() => {
    TestBed.inject(DotnetCaptureStore).stopPolling();
    vi.useRealTimers();
  });

  it('lists processes with runtime, bitness and rights, and explains each kind', async () => {
    const { fixture } = await mount(DotnetDialog);
    const items = all(fixture, 'li.proc');
    expect(items.length).toBe(3);
    expect(items[0]?.getAttribute('aria-selected')).toBe('true');
    expect(items[1]?.textContent).toContain('needs administrator');
    expect(items[1]?.textContent).toContain('x86');
    for (const k of KIND_CHOICES) expect(text(fixture)).toContain(k.help);
    const filter = el<HTMLInputElement>(fixture, '#dotnet-filter');
    filter.value = 'inventory';
    filter.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    expect(all(fixture, 'li.proc').length).toBe(1);
  });

  it('records, shows progress, stops and opens the capture', async () => {
    const { fixture, opener } = await mount(DotnetDialog);
    el<HTMLButtonElement>(fixture, '[data-testid=dotnet-record]').click();
    await settle(fixture, 6);
    expect(all(fixture, 'li.cap').length).toBe(1);
    expect(text(fixture)).toContain('Recording');
    buttonByText(fixture, 'Stop').click();
    await settle(fixture, 6);
    expect(all(fixture, 'li.cap')[0]?.getAttribute('data-state')).toBe('done');
    expect(opener.opened).toEqual([MOCK_PROFILE_PATH]);
  });

  it('offers elevation for .NET Framework processes and shows the failed capture', async () => {
    const { fixture } = await mount(DotnetDialog);
    (all(fixture, 'li.proc')[1] as HTMLElement).click();
    await settle(fixture, 4);
    expect(text(fixture)).toContain('needs administrator rights');
    el<HTMLButtonElement>(fixture, '[data-testid=dotnet-record]').click();
    await settle(fixture, 6);
    expect(all(fixture, 'li.cap')[0]?.getAttribute('data-state')).toBe('failed');
  });

  it('heap snapshots have no duration; cancel and close work', async () => {
    const { fixture, closed } = await mount(DotnetDialog);
    const heap = all<HTMLInputElement>(fixture, 'input[name=dotnet-kind]')[2]!;
    heap.click();
    fixture.detectChanges();
    expect(el<HTMLInputElement>(fixture, '#dotnet-duration').disabled).toBe(true);
    const cpu = all<HTMLInputElement>(fixture, 'input[name=dotnet-kind]')[0]!;
    cpu.click();
    fixture.detectChanges();
    el<HTMLInputElement>(fixture, 'input[type=checkbox]').click();
    fixture.detectChanges();
    const d = el<HTMLInputElement>(fixture, '#dotnet-duration');
    expect(d.disabled).toBe(false);
    d.value = '45';
    d.dispatchEvent(new Event('change'));
    expect(TestBed.inject(DotnetCaptureStore).durationSec()).toBe(45);
    el<HTMLButtonElement>(fixture, '[data-testid=dotnet-record]').click();
    await settle(fixture, 6);
    buttonByText(fixture, 'Cancel').click();
    await settle(fixture, 6);
    expect(all(fixture, 'li.cap')[0]?.getAttribute('data-state')).toBe('cancelled');
    buttonByText(fixture, 'Close').click();
    expect(closed.length).toBe(1);
  });
});

describe('ToolDotnetBackend', () => {
  it('calls the dotnet-profiler tool with each operation', async () => {
    const calls: unknown[][] = [];
    const tools = {
      toolInvoke: (tool: string, op: string, args: unknown) => (calls.push([tool, op, args]), Promise.resolve(op)),
      toolPoll: (tool: string, op: string, args: unknown) => (calls.push(['poll', tool, op, args]), Promise.resolve({ captures: [{ id: 'b', state: 'cancelled' }] })),
      toolTaskJob: (task: string) => (calls.push(['job', task]), Promise.resolve(task === 'b' ? 7 : null)),
    } as unknown as ToolCatalogBackend;
    const jobs = { cancel: (id: number) => (calls.push(['jobs_cancel', id]), Promise.resolve()) } as unknown as JobsBackend;
    const b = new ToolDotnetBackend(tools, jobs);
    await b.processes();
    await b.start({ pid: 1, kind: 'cpu', durationSec: null });
    await b.captures();
    await b.stop('a');
    await b.cancel('b');
    await expect(b.cancel('none')).rejects.toThrow('not running');
    expect(calls).toEqual([
      ['dotnet-profiler', 'processes', {}],
      ['dotnet-profiler', 'start', { pid: 1, kind: 'cpu', durationSec: null }],
      ['poll', 'dotnet-profiler', 'status', {}],
      ['dotnet-profiler', 'stop', { id: 'a' }],
      ['job', 'b'],
      ['jobs_cancel', 7],
      ['poll', 'dotnet-profiler', 'status', {}],
      ['job', 'none'],
    ]);
  });
});
