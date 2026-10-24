import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { CommandContext, DEBRIEF_COMMANDS, DebriefCommand } from '../../extensions/tokens';
import { DialogService } from '../../ui/dialog.service';
import { provideIncident } from './provide-incident';

function setup() {
  TestBed.configureTestingModule({ providers: [provideIncident()] });
  const dialogs = TestBed.inject(DialogService);
  const command = TestBed.inject(DEBRIEF_COMMANDS).flat().find((c: DebriefCommand) => c.id === 'incident.build') as DebriefCommand;
  return { dialogs, command };
}
const ctx = (total: number) => ({ showView: vi.fn(), summary: { total } }) as unknown as CommandContext;

describe('provideIncident', () => {
  it('registers the palette command, offered only with data', () => {
    const { command } = setup();
    expect(command.title).toBe('Report: Incident summary…');
    expect(command.when?.(ctx(0))).toBe(false);
    expect(command.when?.(ctx(5))).toBe(true);
  });

  it('opens the dialog and shows the report view once built', async () => {
    const { command, dialogs } = setup();
    const c = ctx(5);
    const run = command.run(c) as Promise<void>;
    await vi.waitFor(() => expect(dialogs.stack()).toHaveLength(1));
    dialogs.stack().at(0)?.ref.close('show');
    await run;
    expect(c.showView).toHaveBeenCalledWith('report');
  });

  it('does not show the report view when the dialog is cancelled', async () => {
    const { command, dialogs } = setup();
    const c = ctx(5);
    const run = command.run(c) as Promise<void>;
    await vi.waitFor(() => expect(dialogs.stack()).toHaveLength(1));
    dialogs.stack().at(0)?.ref.close();
    await run;
    expect(c.showView).not.toHaveBeenCalled();
  });
});
