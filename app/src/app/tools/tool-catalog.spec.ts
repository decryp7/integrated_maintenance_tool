import { Component, inject } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { pluginInfo, settle } from '../../testing/helpers';
import { ImtBackend } from '../core/backend';
import { ToolCatalogBackend } from '../core/tools-backend';
import { ToolEntry } from '../core/tools.types';
import { WorkspaceStore } from '../core/workspace.store';
import { CommandRegistry } from '../extensions/command-registry';
import { IMT_COMMANDS, IMT_VIEWS } from '../extensions/tokens';
import { ViewRegistry } from '../extensions/view-registry';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { mockTools, provideToolsTesting } from './mock/test-support';
import { MockToolsOptions } from './mock/mock-tools-backend';
import { ToolCatalog } from './tool-catalog.store';
import { ToolUnavailable } from './tool-unavailable';
import { ToolsCommands, toolsViews } from './tools-commands';

@Component({ imports: [ToolUnavailable], template: `<imt-tool-unavailable tool="process-monitor" />` })
class Host {}

async function setup(options: Partial<MockToolsOptions> = {}) {
  TestBed.configureTestingModule({
    providers: [
      ...provideToolsTesting(options),
      { provide: IMT_VIEWS, multi: true, useFactory: () => toolsViews(inject(ToolCatalog)) },
      { provide: IMT_COMMANDS, multi: true, useFactory: () => inject(ToolsCommands).commands },
    ],
  });
  await TestBed.inject(WorkspaceStore).init();
  const catalog = TestBed.inject(ToolCatalog);
  await TestBed.inject(LifecycleService).init();
  await settle();
  return { catalog, views: TestBed.inject(ViewRegistry), commands: TestBed.inject(CommandRegistry) };
}

const viewIds = (views: ViewRegistry) => views.available().map((v) => v.id);
const commandIds = (commands: CommandRegistry) => commands.available().map((c) => c.id);

describe('ToolCatalog', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('lists the tools and offers their views and commands while the plugin is active', async () => {
    const { catalog, views, commands } = await setup();
    expect(catalog.loaded()).toBe(true);
    expect(catalog.entry('perfmon.sets')?.needsElevation).toBe(true);
    expect(catalog.entry('nope')).toBeNull();
    expect(catalog.usable('process-monitor')).toBe(true);
    expect(catalog.problem('process-monitor')).toBeNull();
    expect(viewIds(views)).toEqual(expect.arrayContaining(['perfmon', 'process-monitor', 'plugin-health', 'saved-queries']));
    expect(commandIds(commands)).toEqual(expect.arrayContaining(['tools.perfmon.show', 'tools.monitor.show', 'tools.collect', 'tools.health.show']));
  });

  it('hides tool views and commands in safe mode and when the plugin is disabled, and says why', async () => {
    for (const options of [{ safeMode: true }, { toolsPlugin: 'disabled' as const }]) {
      const { catalog, views, commands } = await setup(options);
      expect(catalog.listed('process-monitor')).toBe(false);
      expect(catalog.usable('process-monitor')).toBe(false);
      expect(catalog.problem('process-monitor')).toEqual({ kind: 'missing' });
      expect(viewIds(views)).not.toContain('perfmon');
      expect(viewIds(views)).not.toContain('process-monitor');
      expect(viewIds(views)).toContain('plugin-health');
      expect(commandIds(commands)).not.toEqual(expect.arrayContaining(['tools.perfmon.show']));
      expect(commandIds(commands)).not.toContain('tools.collect');
      expect(commandIds(commands)).toContain('tools.health.show');
      const fixture = TestBed.createComponent(Host);
      await settle(fixture);
      const text = fixture.nativeElement.textContent as string;
      expect(text).toContain(options.safeMode ? 'off in safe mode' : 'System tools plugin is disabled');
      TestBed.resetTestingModule();
    }
  });

  it('keeps a quarantined plugin\'s tools listed with the engine\'s reason', async () => {
    const { catalog, views } = await setup({ toolsPlugin: 'quarantined' });
    expect(catalog.listed('process-monitor')).toBe(true);
    expect(catalog.usable('process-monitor')).toBe(false);
    expect(catalog.problem('process-monitor')).toEqual({ kind: 'plugin', reason: expect.stringContaining('repeated crashes') });
    expect(viewIds(views)).toContain('process-monitor');
    const fixture = TestBed.createComponent(Host);
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('[data-testid=tool-unavailable]').textContent).toContain('This tool is unavailable: plugin "net.decryptology.tools" is disabled after repeated crashes');
  });

  it('treats an unsupported platform as listed but not usable, without a plugin problem', async () => {
    const { catalog } = await setup({ platform: 'linux' });
    expect(catalog.listed('perfmon.sets')).toBe(true);
    expect(catalog.usable('perfmon.sets')).toBe(false);
    expect(catalog.problem('perfmon.sets')).toBeNull();
  });

  it('is optimistic until the engine answered, and keeps the last answer when a reload fails', async () => {
    let fail = true;
    const entries: ToolEntry[] = [];
    TestBed.configureTestingModule({
      providers: [...provideToolsTesting(), { provide: ToolCatalogBackend, useValue: { toolsList: async () => (fail ? Promise.reject(new Error('no tools_list')) : entries), toolInvoke: async () => null } }],
    });
    const catalog = TestBed.inject(ToolCatalog);
    await settle();
    expect(catalog.error()).toBe('no tools_list');
    expect([catalog.listed('x'), catalog.usable('x'), catalog.problem('x')]).toEqual([true, true, null]);
    fail = false;
    await catalog.load();
    expect([catalog.loaded(), catalog.listed('x'), catalog.error()]).toEqual([true, false, null]);
  });

  it('reads the tools again when a plugin changes state', async () => {
    await setup();
    const spy = vi.spyOn(mockTools(), 'toolsList');
    const imt = TestBed.inject(ImtBackend);
    const summary = await imt.workspaceSummary();
    vi.spyOn(imt, 'workspaceSummary').mockResolvedValue({ ...summary, plugins: [pluginInfo({ id: 'net.decryptology.tools', status: 'quarantined' })] });
    await TestBed.inject(WorkspaceStore).refreshSummary();
    await settle();
    expect(spy).toHaveBeenCalled();
  });
});
