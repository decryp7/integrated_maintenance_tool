import { Type } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { all, buttonByText, el, settle, text } from '../../../testing/helpers';
import { DIALOG_REF } from '../../ui/dialog.service';
import { setupProfiling } from '../testing/profiling-test-support';
import { DotnetBackend, ToolDotnetBackend } from './dotnet-backend';
import { DotnetCaptureStore, DotnetOpener } from './dotnet-capture.store';
import { DotnetDialog, KIND_CHOICES } from './dotnet-dialog';
import { EtlDialog } from './etl-dialog';
import { MOCK_PROFILE_PATH, MockDotnetBackend } from './mock-dotnet-backend';
import { ToolCatalogBackend } from '../../core/tools-backend';
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

describe('EtlDialog', () => {
  it('lists the processes of a trace and opens the chosen one', async () => {
    const { fixture, opener, closed } = await mount(EtlDialog, { path: 'C:\\traces\\shop.etl' });
    expect(text(fixture)).toContain('shop.etl');
    const items = all(fixture, 'li.proc');
    expect(items.length).toBe(3);
    expect(items[0]?.textContent).toContain('.NET');
    (items[2] as HTMLElement).click();
    await settle(fixture, 2);
    el<HTMLButtonElement>(fixture, '[data-testid=etl-open]').click();
    await settle(fixture, 6);
    expect(opener.opened).toEqual([MOCK_PROFILE_PATH]);
    expect(closed.length).toBe(1);
  });

  it('shows errors with Retry', async () => {
    const { fixture, backend } = await mount(EtlDialog, { path: 'C:\\traces\\shop.txt' });
    expect(text(fixture)).toContain('not an .etl file');
    backend.failures.set('etlConvert', 'x');
    (fixture.componentInstance as EtlDialog).load();
    await settle(fixture, 4);
  });
});

describe('ToolDotnetBackend', () => {
  it('calls the dotnet-profiler tool with each operation', async () => {
    const calls: unknown[][] = [];
    const tools = { toolInvoke: (tool: string, op: string, args: unknown) => (calls.push([tool, op, args]), Promise.resolve(op)) } as unknown as ToolCatalogBackend;
    const b = new ToolDotnetBackend(tools);
    await b.processes();
    await b.start({ pid: 1, kind: 'cpu', durationSec: null });
    await b.captures();
    await b.stop('a');
    await b.cancel('b');
    await b.etlProcesses('x.etl');
    await b.etlConvert('x.etl', 3);
    expect(calls).toEqual([
      ['dotnet-profiler', 'processes', {}],
      ['dotnet-profiler', 'start', { pid: 1, kind: 'cpu', durationSec: null }],
      ['dotnet-profiler', 'status', {}],
      ['dotnet-profiler', 'stop', { id: 'a' }],
      ['dotnet-profiler', 'cancel', { id: 'b' }],
      ['dotnet-profiler', 'etl-processes', { path: 'x.etl' }],
      ['dotnet-profiler', 'etl-convert', { path: 'x.etl', pid: 3 }],
    ]);
  });
});
