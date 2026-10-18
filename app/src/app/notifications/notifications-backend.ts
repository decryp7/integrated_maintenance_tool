import { inject, Injectable, Injector } from '@angular/core';
import { invoke } from '../core/activity/ipc';
import { reportBackendFailures } from '../core/failure-reporter';
import { Logger, withFailureLogging } from '../core/logger';
import { isTauri } from '../core/platform';
import { listenHere, PayloadListenFn, Unlisten } from '../windowing/window-events';
import { NoticesEvent, NoticesRecord, NoticesSnapshot, RestoredNotices } from './notifications.types';

export const NOTIFICATIONS_EVENT = 'imt://notifications';
export const NOTIFICATIONS_OPEN_EVENT = 'imt://notifications-open';

/**
 * The notification centre of this window (contract: `notifications.types.ts`). Injected as a class token: the
 * Tauri commands in the desktop app, the mock in the browser build.
 */
@Injectable({ providedIn: 'root', useFactory: () => createNotificationsBackend() })
export abstract class NotificationsBackend {
  abstract list(): Promise<NoticesSnapshot>;
  abstract markRead(id: number): Promise<boolean>;
  abstract markAllRead(): Promise<number>;
  abstract dismiss(id: number): Promise<boolean>;
  abstract record(): Promise<NoticesRecord>;
  abstract restore(record: unknown): Promise<RestoredNotices>;
  abstract onChange(handler: (e: NoticesEvent) => void): Promise<Unlisten>;
  /** An OS notification was clicked: open that message. */
  abstract onOpen(handler: (id: number) => void): Promise<Unlisten>;
}

const isEvent = (p: unknown): p is NoticesEvent => !!p && typeof p === 'object' && Array.isArray((p as NoticesEvent).notices) && typeof (p as NoticesEvent).unread === 'number';

export class TauriNotificationsBackend implements NotificationsBackend {
  constructor(
    private readonly call: typeof invoke = invoke,
    private readonly listen: PayloadListenFn = listenHere,
  ) {}

  list() {
    return this.call<NoticesSnapshot>('notifications_list');
  }
  markRead(id: number) {
    return this.call<boolean>('notifications_mark_read', { id });
  }
  markAllRead() {
    return this.call<number>('notifications_mark_all_read');
  }
  dismiss(id: number) {
    return this.call<boolean>('notifications_dismiss', { id });
  }
  record() {
    return this.call<NoticesRecord>('notifications_record');
  }
  restore(record: unknown) {
    return this.call<RestoredNotices>('notifications_restore', { record });
  }
  onChange(handler: (e: NoticesEvent) => void) {
    return this.listen(NOTIFICATIONS_EVENT, (p) => {
      if (isEvent(p)) handler({ ...p, posted: p.posted ?? null });
    });
  }
  onOpen(handler: (id: number) => void) {
    return this.listen(NOTIFICATIONS_OPEN_EVENT, (p) => {
      if (typeof p === 'number') handler(p);
    });
  }
}

let shared: { injector: Injector; backend: NotificationsBackend } | null = null;

export function createNotificationsBackend(): NotificationsBackend {
  const injector = inject(Injector);
  if (shared?.injector !== injector) {
    const target: NotificationsBackend = isTauri() ? new TauriNotificationsBackend() : lazyMock();
    shared = { injector, backend: withFailureLogging<NotificationsBackend>(target, inject(Logger), { onFailure: reportBackendFailures() }) };
  }
  return shared.backend;
}

function lazyMock(): NotificationsBackend {
  let loaded: Promise<NotificationsBackend> | null = null;
  const get = () => (loaded ??= import('./mock-notifications').then((m) => m.mockNotifications()));
  return new Proxy({} as NotificationsBackend, {
    get(_t, name) {
      if (typeof name !== 'string' || name === 'then' || name.startsWith('ng')) return undefined;
      return (...args: unknown[]) =>
        get().then((b) => {
          const fn = (b as unknown as Record<string, (...a: unknown[]) => Promise<unknown>>)[name];
          if (!fn) throw new TypeError(`${name} is not a function`);
          return fn.apply(b, args);
        });
    },
  });
}
