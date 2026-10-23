import { ComponentFixture, TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';
import { all, el, settle, text, typeInto } from '../../testing/helpers';
import { ToolCatalog } from '../tools/tool-catalog.store';
import { CaptureProcess, CreateDumpView, filterProcesses } from './create-dump-view';
import { DumpsStore } from './dumps.store';
import { setupDumps } from './testing/dumps-test-support';
import { MockToolsOptions } from '../tools/mock/mock-tools-backend';

async function mount(tools: Partial<MockToolsOptions> = {}) {
  const t = setupDumps({ tools });
  await TestBed.inject(ToolCatalog).load();
  const fixture: ComponentFixture<CreateDumpView> = TestBed.createComponent(CreateDumpView);
  fixture.detectChanges();
  await settle(fixture, 6);
  return { fixture, t, view: fixture.componentInstance, store: TestBed.inject(DumpsStore) };
}

const procs: CaptureProcess[] = [
  { pid: 4120, name: 'OrderService.exe', parent: 1, threads: 3 },
  { pid: 77, name: 'w3wp.exe', parent: 1, threads: 3 },
];

describe('filterProcesses', () => {
  it('matches names and pids', () => {
    expect(filterProcesses(procs, ' ORDER ').map((p) => p.pid)).toEqual([4120]);
    expect(filterProcesses(procs, '77').map((p) => p.pid)).toEqual([77]);
    expect(filterProcesses(procs, '')).toHaveLength(2);
  });
});

describe('CreateDumpView', () => {
  it('writes a dump of the chosen process into the chosen folder and opens it', async () => {
    const { fixture, t, view, store } = await mount();
    const options = all(fixture, '[data-testid="capture-processes"] [role="option"]');
    expect(options.length).toBe(5);
    typeInto(el<HTMLInputElement>(fixture, 'input[type="search"]'), 'orderservice');
    await settle(fixture, 4);
    const shown = all(fixture, '[data-testid="capture-processes"] [role="option"]');
    expect(shown).toHaveLength(2);
    expect(el<HTMLButtonElement>(fixture, '[data-testid="capture-create"]').disabled).toBe(true);
    shown[0]!.click();
    el<HTMLInputElement>(fixture, '[data-testid="capture-kind-full"]').dispatchEvent(new Event('change'));
    el<HTMLButtonElement>(fixture, '[data-testid="capture-folder"]').click();
    await settle(fixture, 4);
    expect(view.pid()).toBe(4120);
    expect(view.kind()).toBe('full');
    el<HTMLButtonElement>(fixture, '[data-testid="capture-create"]').click();
    await settle(fixture, 8);
    expect(el(fixture, '[data-testid="capture-done"]').textContent).toContain('C:\\Dumps\\OrderService.exe.4120.full.20261014-104124.dmp');
    expect(store.dumps()).toHaveLength(1);
    expect(t.context.last('showView')).toEqual(['dump']);
    view.pickPid('x');
    expect(view.pid()).toBe(4120);
  });

  it('shows a refusal with the elevation hint, and nothing to do without a folder', async () => {
    const { fixture, t, view } = await mount();
    view.pickPid(4);
    await view.create();
    t.folders.folder = null;
    await view.chooseFolder();
    expect(view.folder()).toBeNull();
    t.folders.folder = 'C:\\Dumps';
    await view.chooseFolder();
    await view.create();
    await settle(fixture, 4);
    expect(text(fixture)).toContain('Restart Debrief as administrator');
  });

  it('says when the tool cannot run on this platform', async () => {
    const { fixture } = await mount({ platform: 'linux' });
    expect(el(fixture, '[data-testid="capture-unsupported"]').textContent).toContain('only available on Windows');
  });

  it('shows a failed process list with Retry', async () => {
    const { fixture, view } = await mount({ toolsPlugin: 'quarantined' });
    expect(text(fixture)).toContain('This tool is unavailable');
    expect(view.error()).toContain('disabled after repeated crashes');
  });
});
