import { TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';
import { DialogService } from '../../ui/dialog.service';
import { setupProfiling } from '../testing/profiling-test-support';
import { DotnetCommands } from './dotnet-commands';

class FakeDialogs {
  opened: { title: string; inputs?: Record<string, unknown> }[] = [];
  open(_: unknown, options: { title: string; inputs?: Record<string, unknown> }) {
    this.opened.push(options);
    return { close: () => undefined, closed: Promise.resolve(undefined) };
  }
}

describe('DotnetCommands', () => {
  it('opens the capture dialog; .etl traces are the native profiler’s', async () => {
    setupProfiling();
    const dialogs = new FakeDialogs();
    TestBed.overrideProvider(DialogService, { useValue: dialogs });
    const commands = TestBed.inject(DotnetCommands);
    expect(commands.commands.map((c) => c.id)).toEqual(['profiling.dotnet']);
    await commands.commands[0]!.run({} as never);
    await commands.openCapture();
    expect(dialogs.opened.some((d) => d.title === 'Profile .NET process')).toBe(true);
  });
});
