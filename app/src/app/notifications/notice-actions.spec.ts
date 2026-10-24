import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { JobsStore } from '../jobs/jobs.store';
import { SettingsDialogService } from '../settings/settings-dialog';
import { actionLabel, NoticeActions } from './notice-actions';
import { NotificationsStore } from './notifications.store';
import { Notice } from './notifications.types';

describe('NoticeActions: openSettings', () => {
  it('opens Settings at the section the message names (a service the user fixes there)', async () => {
    const open = vi.fn(async () => undefined);
    const markRead = vi.fn(async () => undefined);
    TestBed.configureTestingModule({
      providers: [
        { provide: SettingsDialogService, useValue: { open } },
        { provide: NotificationsStore, useValue: { markRead, dismiss: vi.fn() } },
        { provide: JobsStore, useValue: { loaded: () => false } },
        { provide: COMMAND_CONTEXT, useValue: { showView: () => undefined } },
      ],
    });
    const action = { type: 'openSettings', section: 'browser' } as const;
    expect(actionLabel(action)).toBe('Open Settings…');
    const err = await TestBed.inject(NoticeActions).run({ id: 7 } as Notice, action);
    expect(err).toBeNull();
    expect(open).toHaveBeenCalledWith('browser');
    expect(markRead).toHaveBeenCalledWith(7);
  });
});
