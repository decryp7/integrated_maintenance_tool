import { TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';
import { DialogService } from '../../ui/dialog.service';
import { setupProfiling } from '../testing/profiling-test-support';
import { DotnetCommands, EtlPicker } from './dotnet-commands';

class FakeDialogs {
  opened: { title: string; inputs?: Record<string, unknown> }[] = [];
  open(_: unknown, options: { title: string; inputs?: Record<string, unknown> }) {
    this.opened.push(options);
    return { close: () => undefined, closed: Promise.resolve(undefined) };
  }
}

function setup(path: string | null) {
  setupProfiling();
  const dialogs = new FakeDialogs();
  TestBed.overrideProvider(DialogService, { useValue: dialogs });
  TestBed.overrideProvider(EtlPicker, { useValue: { pick: () => Promise.resolve(path) } });
  return { commands: TestBed.inject(DotnetCommands), dialogs };
}

describe('DotnetCommands', () => {
  it('opens the capture dialog and the .etl dialog with the picked file', async () => {
    const { commands, dialogs } = setup('C:\\t\\a.etl');
    expect(commands.commands.map((c) => c.id)).toEqual(['profiling.dotnet', 'profiling.etl']);
    await commands.commands[0]!.run({} as never);
    await commands.commands[1]!.run({} as never);
    // run() does not wait for the lazy dialogs: call the methods directly too.
    await commands.openCapture();
    await commands.openEtl();
    expect(dialogs.opened.some((d) => d.title === 'Profile .NET process')).toBe(true);
    expect(dialogs.opened.find((d) => d.inputs)?.inputs).toEqual({ path: 'C:\\t\\a.etl' });
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
