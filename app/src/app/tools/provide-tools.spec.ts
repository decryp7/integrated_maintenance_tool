import { ApplicationConfig } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { settle } from '../../testing/helpers';
import { provideBackend } from '../core/backend';
import { Logger } from '../core/logger';
import { MemoryLogger } from '../../testing/helpers';
import { provideDebriefBuiltins } from '../builtin/provide-builtins';
import { DockService } from '../dock/dock.service';
import { CommandRegistry } from '../extensions/command-registry';
import { ViewRegistry } from '../extensions/view-registry';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { SessionService } from '../lifecycle/session.service';
import { UiStateRegistry } from '../lifecycle/ui-state';
import { DialogService } from '../ui/dialog.service';
import { FakeDock } from '../../testing/helpers';
import { CollectorDialog } from './collector/collector-dialog';
import { provideTools, startTools } from './provide-tools';
import { toolsViews } from './tools-commands';

const TOOLS_VIEWS = toolsViews({ listed: () => true });

function configure() {
  TestBed.configureTestingModule({
    providers: [
      { provide: Logger, useFactory: () => new MemoryLogger() },
      { provide: DockService, useValue: new FakeDock() },
      provideBackend(),
      provideDebriefBuiltins(),
      provideTools(),
    ],
  });
}

describe('provideTools', () => {
  afterEach(() => {
    document.body.innerHTML = '';
    TestBed.resetTestingModule();
  });

  it('is a single provider entry for the app config', () => {
    const config: ApplicationConfig = { providers: [provideTools()] };
    expect(config.providers).toHaveLength(1);
  });

  it('registers the tool views, commands and UI state contributions', async () => {
    configure();
    const views = TestBed.inject(ViewRegistry);
    expect(TOOLS_VIEWS.map((v) => v.id).every((id) => !!views.find(id))).toBe(true);
    for (const v of TOOLS_VIEWS) expect(typeof (await v.component())).toBe('function');
    const commands = TestBed.inject(CommandRegistry);
    const ids = commands.all().map((c) => c.id);
    expect(ids).toEqual(
      expect.arrayContaining(['tools.perfmon.show', 'tools.monitor.show', 'tools.health.show', 'tools.collect', 'app.restartElevated', 'project.new', 'project.open', 'project.save', 'project.saveAs', 'project.recent', 'savedQueries.show']),
    );
    await settle();
    expect(Object.keys(TestBed.inject(UiStateRegistry).serialize())).toEqual(expect.arrayContaining(['query', 'layout', 'selection', 'savedQueries', 'highlightRules']));
  });

  it('opens views and the collector dialog from commands', async () => {
    configure();
    const commands = TestBed.inject(CommandRegistry);
    await settle();
    const dock = TestBed.inject(DockService) as unknown as FakeDock;
    await commands.run('tools.perfmon.show');
    await commands.run('tools.monitor.show');
    await commands.run('tools.health.show');
    expect(dock.calls).toEqual(['show:perfmon', 'show:process-monitor', 'show:plugins']);
    await commands.run('tools.collect');
    expect(TestBed.inject(DialogService).stack()[0]?.component).toBe(CollectorDialog);
  });

  it('starts the lifecycle in the background: status and autosave, without touching the DOM', async () => {
    configure();
    TestBed.runInInjectionContext(() => startTools());
    await settle();
    await settle();
    expect(TestBed.inject(LifecycleService).status()).not.toBeNull();
    expect(TestBed.inject(SessionService)).toBeDefined();
    expect(document.body.querySelector('debrief-title-extras, debrief-lifecycle-banners')).toBeNull();
  });
});
