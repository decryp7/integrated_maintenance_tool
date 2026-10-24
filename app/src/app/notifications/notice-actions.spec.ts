import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { JobsStore } from '../jobs/jobs.store';
import { SettingsDialogService } from '../settings/settings-dialog';
import { FakeDock, provideFakeDock } from '../../testing/helpers';
import { ServiceHealthStore } from '../health/service-health.store';
import { actionLabel, actionTarget, NoticeActions, NOTIFICATIONS_VIEW } from './notice-actions';
import { NotificationsBackend } from './notifications-backend';
import { NoticeTarget, NotificationsStore } from './notifications.store';
import { EMPTY_NOTICES, Notice, NoticeAction } from './notifications.types';

describe('NoticeActions: openSettings', () => {
  it('opens Settings at the section the message names (a service the user fixes there)', async () => {
    const open = vi.fn(async () => undefined);
    const markRead = vi.fn(async () => undefined);
    TestBed.configureTestingModule({
      providers: [
        provideFakeDock(),
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

describe('NoticeActions: leaving Notifications for what an action opens', () => {
  function setup(err: string | null = null) {
    const dock = new FakeDock();
    const showView = vi.fn(() => {
      if (err) throw new Error(err);
    });
    TestBed.configureTestingModule({
      providers: [
        provideFakeDock(dock),
        { provide: JobsStore, useValue: { loaded: () => false } },
        { provide: COMMAND_CONTEXT, useValue: { showView } },
        { provide: ServiceHealthStore, useValue: { repair: vi.fn(async () => undefined) } },
        { provide: NotificationsBackend, useValue: { markRead: vi.fn(async () => true), list: async () => EMPTY_NOTICES, onChange: async () => () => undefined, onOpen: async () => () => undefined } },
      ],
    });
    return { dock, actions: TestBed.inject(NoticeActions), store: TestBed.inject(NotificationsStore) };
  }
  const leaves = (dock: FakeDock) => dock.calls.filter((c) => c === `slideIn:${NOTIFICATIONS_VIEW}` || c === 'focusFront');

  it('says where every action goes: a view, a window of its own, or nowhere (it acts in place)', () => {
    const want: Record<NoticeAction['type'], NoticeTarget> = {
      viewJob: 'view',
      reviewProposals: 'view',
      retryJob: 'view',
      showJobs: 'view',
      openAssistant: 'view',
      setUpAssistant: 'view',
      updateAi: 'view',
      showAlert: 'view',
      setUpFormat: 'window',
      fixTimestamps: 'window',
      openSettings: 'window',
      showInFolder: null,
      resetPlugin: null,
      resetAi: null,
      installUpdate: null,
      laterAiUpdate: null,
      skipAiUpdate: null,
      repairService: null,
    };
    for (const [type, target] of Object.entries(want)) expect(actionTarget({ type } as NoticeAction), type).toBe(target);
  });

  it('an action that opens a view slides Notifications in and moves the focus to the view', async () => {
    const { dock, actions } = setup();
    expect(await actions.run({ id: -1 } as Notice, { type: 'showJobs' })).toBeNull();
    expect(leaves(dock)).toEqual([`slideIn:${NOTIFICATIONS_VIEW}`, 'focusFront']);
  });

  it('an action that acts in place (Try again) keeps Notifications open', async () => {
    const { dock, actions } = setup();
    expect(await actions.run({ id: -1 } as Notice, { type: 'repairService', service: 'store' })).toBeNull();
    expect(leaves(dock)).toEqual([]);
  });

  it('a failed action keeps Notifications open, with its error', async () => {
    const { dock, actions } = setup('no such view');
    expect(await actions.run({ id: -1 } as Notice, { type: 'showJobs' })).toBe('no such view');
    expect(leaves(dock)).toEqual([]);
  });

  it("a message of the UI's own declares where it goes: a view, a window, or nowhere", () => {
    const { dock, actions, store } = setup();
    const post = (key: string, target: NoticeTarget) => {
      store.postLocal({ key, source: 'workspace', severity: 'info', title: key, actionLabel: 'Go', target, run: () => undefined });
      return store.local().find((l) => l.notice.cause === 'local:' + key)!.notice.id;
    };
    actions.runLocal(post('stay', null));
    expect(leaves(dock)).toEqual([]);
    actions.runLocal(post('window', 'window'));
    expect(leaves(dock)).toEqual([`slideIn:${NOTIFICATIONS_VIEW}`]);
    actions.runLocal(post('view', 'view'));
    expect(leaves(dock)).toEqual([`slideIn:${NOTIFICATIONS_VIEW}`, `slideIn:${NOTIFICATIONS_VIEW}`, 'focusFront']);
    expect(store.local()).toEqual([]);
  });
});
