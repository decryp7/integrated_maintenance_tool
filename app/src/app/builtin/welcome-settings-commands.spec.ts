import { TestBed } from '@angular/core/testing';
import { provideFakeDock, provideTinyBackend, settle } from '../../testing/helpers';
import { CommandRegistry } from '../extensions/command-registry';
import { DialogService } from '../ui/dialog.service';
import { WelcomeService } from '../welcome/welcome';

describe('welcome, settings and export commands', () => {
  function setup() {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock()] });
    return TestBed.inject(CommandRegistry);
  }

  it('shows the welcome screen', async () => {
    const registry = setup();
    await settle();
    expect(await registry.run('help.welcome')).toBe(true);
    expect(TestBed.inject(WelcomeService).visible()).toBe(true);
  });

  it('opens the settings dialog', async () => {
    const registry = setup();
    await settle();
    await registry.run('settings.open');
    expect(TestBed.inject(DialogService).stack()[0]?.options.title).toBe('Settings');
  });

  it('opens the settings dialog at the Diagnostics section', async () => {
    const registry = setup();
    await settle();
    await registry.run('settings.diagnostics');
    expect(TestBed.inject(DialogService).stack()[0]?.options.title).toBe('Settings');
    // The section travels to the dialog window as its input.
    expect(TestBed.inject(DialogService).stack()[0]?.options.inputs).toEqual({ section: 'diagnostics' });
  });

  it('opens the export dialog only when exporters exist', async () => {
    const registry = setup();
    expect(await registry.run('file.export')).toBe(false);
    await settle();
    const done = registry.run('file.export');
    await TestBed.inject(DialogService).whenLoaded();
    expect(TestBed.inject(DialogService).stack()[0]?.options.title).toBe('Export events');
    TestBed.inject(DialogService).dismissTop();
    expect(await done).toBe(true);
  });
});
