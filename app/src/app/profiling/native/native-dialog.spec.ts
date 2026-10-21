import { ComponentFixture, TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { all, buttonByText, el, settle, text } from '../../../testing/helpers';
import { ToolCatalogBackend } from '../../core/tools-backend';
import { JobsBackend } from '../../jobs/jobs-backend';
import { provideToolsTesting } from '../../tools/mock/test-support';
import { DIALOG_REF, DialogService } from '../../ui/dialog.service';
import { EtlDialog } from './etl-dialog';
import { setupProfiling } from '../testing/profiling-test-support';
import { NativeBackend, ToolNativeBackend, toolOf } from './native-backend';
import { NativeCaptureStore, NativeOpener } from './native-capture.store';
import { EtlPicker, NativeCommands } from './native-commands';
import { MODE_CHOICES, NativeDialog } from './native-dialog';
import { MOCK_NATIVE_PROFILE_PATH, MockNativeBackend } from './mock-native-backend';

class FakeOpener {
  opened: string[] = [];
  async open(path: string) {
    this.opened.push(path);
  }
}

async function mount() {
  const backend = new MockNativeBackend();
  const opener = new FakeOpener();
  const closed: unknown[] = [];
  setupProfiling();
  TestBed.overrideProvider(NativeBackend, { useValue: backend });
  TestBed.overrideProvider(NativeOpener, { useValue: opener });
  TestBed.configureTestingModule({ providers: [...provideToolsTesting(), { provide: DIALOG_REF, useValue: { close: (r?: unknown) => closed.push(r ?? 'closed'), closed: Promise.resolve(undefined) } }] });
  const fixture: ComponentFixture<NativeDialog> = TestBed.createComponent(NativeDialog);
  fixture.detectChanges();
  await settle(fixture, 6);
  return { fixture, backend, opener, closed };
}

describe('NativeDialog', () => {
  beforeEach(() => vi.useFakeTimers({ shouldAdvanceTime: true }));
  afterEach(() => {
    TestBed.inject(NativeCaptureStore).stopPolling();
    vi.useRealTimers();
  });

  it('lists processes with architecture and rights, explains both modes and prefills the symbol path', async () => {
    const { fixture } = await mount();
    const items = all(fixture, 'li.proc');
    expect(items.length).toBe(3);
    expect(items[0]?.getAttribute('aria-selected')).toBe('true');
    expect(items[1]?.textContent).toContain('x86');
    expect(items[2]?.textContent).toContain('ETW only');
    for (const m of MODE_CHOICES) expect(text(fixture)).toContain(m.help);
    expect(el<HTMLInputElement>(fixture, '#native-symbols').value).toBe('C:\\symbols');
    const filter = el<HTMLInputElement>(fixture, '#native-filter');
    filter.value = 'order';
    filter.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    expect(all(fixture, 'li.proc').length).toBe(1);
  });

  it('samples, shows progress, stops and opens the capture', async () => {
    const { fixture, opener } = await mount();
    const rate = el<HTMLInputElement>(fixture, '#native-rate');
    rate.value = '250';
    rate.dispatchEvent(new Event('change'));
    expect(TestBed.inject(NativeCaptureStore).rateHz()).toBe(250);
    el<HTMLButtonElement>(fixture, '[data-testid=native-record]').click();
    await settle(fixture, 6);
    expect(all(fixture, 'li.cap').length).toBe(1);
    expect(text(fixture)).toContain('Recording');
    expect(text(fixture)).toContain('samples');
    buttonByText(fixture, 'Stop').click();
    await settle(fixture, 6);
    expect(all(fixture, 'li.cap')[0]?.getAttribute('data-state')).toBe('done');
    expect(text(fixture)).toContain('modules had a PDB');
    expect(opener.opened).toEqual([MOCK_NATIVE_PROFILE_PATH]);
  });

  it('points processes the sampler cannot open to ETW; ETW offers heap allocations', async () => {
    const { fixture } = await mount();
    (all(fixture, 'li.proc')[2] as HTMLElement).click();
    await settle(fixture, 4);
    expect(text(fixture)).toContain('Choose ETW (administrator)');
    expect(el<HTMLButtonElement>(fixture, '[data-testid=native-record]').disabled).toBe(true);
    all<HTMLInputElement>(fixture, 'input[name=native-mode]')[1]!.click();
    fixture.detectChanges();
    const heap = el<HTMLInputElement>(fixture, 'input[type=checkbox]');
    heap.click();
    fixture.detectChanges();
    expect(TestBed.inject(NativeCaptureStore).heap()).toBe(true);
    el<HTMLButtonElement>(fixture, '[data-testid=native-record]').click();
    await settle(fixture, 6);
    expect(all(fixture, 'li.cap')[0]?.textContent).toContain('ETW');
  });

  it('a declined administrator prompt shows the restart hint; cancel, duration and close work', async () => {
    const { fixture, backend, closed } = await mount();
    all<HTMLInputElement>(fixture, 'input[name=native-mode]')[1]!.click();
    fixture.detectChanges();
    backend.declineElevation = true;
    el<HTMLButtonElement>(fixture, '[data-testid=native-record]').click();
    await settle(fixture, 6);
    expect(text(fixture)).toContain('declined');
    expect(TestBed.inject(NativeCaptureStore).errorCode()).toBe('denied');
    backend.declineElevation = false;
    all<HTMLInputElement>(fixture, 'input[name=native-mode]')[0]!.click();
    fixture.detectChanges();
    const duration = el<HTMLInputElement>(fixture, '#native-duration');
    duration.value = '45';
    duration.dispatchEvent(new Event('change'));
    expect(TestBed.inject(NativeCaptureStore).durationSec()).toBe(45);
    const stopAfter = all<HTMLInputElement>(fixture, 'input[type=checkbox]')[0]!;
    stopAfter.click();
    fixture.detectChanges();
    expect(TestBed.inject(NativeCaptureStore).durationSec()).toBeNull();
    const server = all<HTMLInputElement>(fixture, 'input[type=checkbox]').at(-1)!;
    server.click();
    expect(TestBed.inject(NativeCaptureStore).symbolServer()).toBe(true);
    const symbols = el<HTMLInputElement>(fixture, '#native-symbols');
    symbols.value = 'D:\\pdb';
    symbols.dispatchEvent(new Event('change'));
    expect(TestBed.inject(NativeCaptureStore).symbolPath()).toBe('D:\\pdb');
    el<HTMLButtonElement>(fixture, '[data-testid=native-record]').click();
    await settle(fixture, 6);
    buttonByText(fixture, 'Cancel').click();
    await settle(fixture, 6);
    expect(all(fixture, 'li.cap')[0]?.getAttribute('data-state')).toBe('cancelled');
    buttonByText(fixture, 'Close').click();
    expect(closed.length).toBe(1);
  });
});

describe('EtlDialog', () => {
  async function mountEtl(path: string) {
    const backend = new MockNativeBackend();
    const opener = new FakeOpener();
    const closed: unknown[] = [];
    setupProfiling();
    TestBed.configureTestingModule({ providers: [...provideToolsTesting(), { provide: DIALOG_REF, useValue: { close: (r?: unknown) => closed.push(r ?? 'closed'), closed: Promise.resolve(undefined) } }] });
    TestBed.overrideProvider(NativeBackend, { useValue: backend });
    TestBed.overrideProvider(NativeOpener, { useValue: opener });
    const fixture = TestBed.createComponent(EtlDialog);
    fixture.componentRef.setInput('path', path);
    fixture.detectChanges();
    await settle(fixture, 6);
    return { fixture, backend, opener, closed };
  }

  it('lists the processes of a trace, .NET ones first, and opens the chosen one', async () => {
    const { fixture, opener, closed } = await mountEtl('C:/traces/shop.etl');
    expect(text(fixture)).toContain('shop.etl');
    const items = all(fixture, 'li.proc');
    expect(items.length).toBe(3);
    expect(items[0]?.textContent).toContain('.NET');
    (items[1] as HTMLElement).click();
    await settle(fixture, 2);
    el<HTMLButtonElement>(fixture, '[data-testid=etl-open]').click();
    await settle(fixture, 6);
    expect(opener.opened).toEqual([MOCK_NATIVE_PROFILE_PATH]);
    expect(closed.length).toBe(1);
  });

  it('shows errors with Retry', async () => {
    const { fixture, backend } = await mountEtl('C:/traces/shop.txt');
    expect(text(fixture)).toContain('not an .etl file');
    backend.failures.set('etlConvert', 'x');
    await (fixture.componentInstance as EtlDialog).load();
    await settle(fixture, 4);
  });
});

class FakeDialogs {
  opened: { title: string; inputs?: Record<string, unknown> }[] = [];
  open(_: unknown, options: { title: string; inputs?: Record<string, unknown> }) {
    this.opened.push(options);
    return { close: () => undefined, closed: Promise.resolve(undefined) };
  }
}

describe('NativeCommands', () => {
  function setup(path: string | null) {
    setupProfiling();
    const dialogs = new FakeDialogs();
    TestBed.overrideProvider(DialogService, { useValue: dialogs });
    TestBed.overrideProvider(EtlPicker, { useValue: { pick: () => Promise.resolve(path) } });
    TestBed.overrideProvider(NativeBackend, { useValue: new MockNativeBackend() });
    return { commands: TestBed.inject(NativeCommands), dialogs };
  }

  it('opens the capture dialog and the one .etl dialog with the picked file', async () => {
    const { commands, dialogs } = setup('C:\\t\\a.etl');
    expect(commands.commands.map((c) => c.id)).toEqual(['profiling.native', 'profiling.etl']);
    await commands.commands[0]!.run({} as never);
    await commands.commands[1]!.run({} as never);
    await commands.openCapture();
    await commands.openEtl();
    expect(dialogs.opened.some((d) => d.title === 'Profile native process')).toBe(true);
    const etl = dialogs.opened.find((d) => d.inputs);
    expect(etl?.title).toBe('Open a process from an ETW trace');
    expect(etl?.inputs).toEqual({ path: 'C:\\t\\a.etl' });
  });

  it('does nothing when no .etl file was chosen', async () => {
    const { commands, dialogs } = setup(null);
    await commands.openEtl();
    expect(dialogs.opened).toEqual([]);
  });

  it('offers a demo trace in the browser', async () => {
    setupProfiling();
    expect(await TestBed.inject(EtlPicker).pick()).toBe('/demo/traces/shop.etl');
  });
});

describe('ToolNativeBackend', () => {
  it('calls each tool with its operations; ETW goes to the elevated tool; cancel goes through the job', async () => {
    const calls: unknown[][] = [];
    const tools = {
      toolInvoke: (tool: string, op: string, args: unknown) => (calls.push([tool, op, args]), Promise.resolve(op)),
      toolPoll: (tool: string, op: string, args: unknown) => (calls.push(['poll', tool, op, args]), Promise.resolve({ captures: [{ id: tool === 'native-profiler' ? 'b' : 'e', state: 'cancelled' }] })),
      toolTaskJob: (task: string) => (calls.push(['job', task]), Promise.resolve(task === 'b' ? 7 : null)),
    } as unknown as ToolCatalogBackend;
    const jobs = { cancel: (id: number) => (calls.push(['jobs_cancel', id]), Promise.resolve()) } as unknown as JobsBackend;
    const b = new ToolNativeBackend(tools, jobs);
    await b.defaults();
    await b.processes();
    await b.start('sampling', { pid: 1, rateHz: 100 });
    await b.start('etw', { pid: 2, heap: true });
    expect((await b.captures(['sampling', 'etw'])).map((c) => c.id)).toEqual(['b', 'e']);
    await b.stop('etw', 'e');
    await b.cancel('sampling', 'b');
    await b.etlProcesses('x.etl');
    await b.etlConvert('x.etl', 3);
    await expect(b.cancel('sampling', 'none')).rejects.toThrow('not running');
    await expect(b.cancel('etw', 'b')).rejects.toThrow('no longer listed');
    expect(calls.slice(0, 4)).toEqual([
      ['native-profiler', 'defaults', {}],
      ['native-profiler', 'processes', {}],
      ['native-profiler', 'start', { pid: 1, rateHz: 100 }],
      ['native-profiler.etw', 'start', { pid: 2, heap: true }],
    ]);
    expect(calls).toContainEqual(['native-profiler.etw', 'stop', { id: 'e' }]);
    expect(calls).toContainEqual(['jobs_cancel', 7]);
    expect(calls).toContainEqual(['native-profiler', 'etl-convert', { path: 'x.etl', pid: 3 }]);
    expect(toolOf('etw')).toBe('native-profiler.etw');
  });
});
