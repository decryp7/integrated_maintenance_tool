import { Unlisten } from '../windowing/window-events';
import { NotificationsBackend } from './notifications-backend';
import { Notice, NoticeAction, NoticeLink, NoticeScope, NoticeSeverity, NoticeSource, NoticesEvent, NoticesRecord, NoticesSnapshot, RestoredNotices } from './notifications.types';

/** What a mock producer posts (the engine's `NewNotice`). */
export interface MockNewNotice {
  source: NoticeSource;
  severity: NoticeSeverity;
  title: string;
  body?: string;
  primary?: NoticeAction | null;
  secondary?: NoticeAction | null;
  dedupeKey?: string | null;
  scope?: NoticeScope;
  link?: Partial<NoticeLink>;
}

const CAP = 200;

/**
 * The notification centre for the browser build: it mimics the engine's rules (dedupe with counts, caps
 * trimming read messages first) closely enough for the UI and e2e tests. It is not the reference.
 */
export class MockNotifications implements NotificationsBackend {
  private notices: Notice[] = [];
  private next = 1;
  private readonly handlers = new Set<(e: NoticesEvent) => void>();
  private readonly openHandlers = new Set<(id: number) => void>();

  private snapshot(): NoticesSnapshot {
    const notices = [...this.notices].sort((a, b) => b.at - a.at || b.id - a.id);
    return { notices, unread: notices.filter((n) => !n.read).length };
  }

  private emit(posted: Notice | null): void {
    const e: NoticesEvent = { ...this.snapshot(), posted };
    for (const h of this.handlers) h(e);
  }

  /** Posts a message (as the engine's producers do). */
  post(n: MockNewNotice): Notice {
    const now = Date.now();
    const key = n.dedupeKey ?? null;
    const existing = key ? this.notices.find((m) => m.dedupeKey === key) : undefined;
    let notice: Notice;
    if (existing) {
      notice = { ...existing, count: existing.count + 1, at: now, read: false, title: n.title, body: n.body ?? '', severity: n.severity, primary: n.primary ?? null, secondary: n.secondary ?? null };
      this.notices = [...this.notices.filter((m) => m !== existing), notice];
    } else {
      notice = {
        id: this.next++,
        source: n.source,
        severity: n.severity,
        title: n.title,
        body: n.body ?? '',
        at: now,
        firstAt: now,
        count: 1,
        read: false,
        primary: n.primary ?? null,
        secondary: n.secondary ?? null,
        link: { window: 'main', jobId: null, recordId: null, pluginId: null, path: null, ...n.link },
        dedupeKey: key,
        scope: n.scope ?? 'project',
      };
      this.notices = [...this.notices, notice];
      while (this.notices.length > CAP) {
        const i = this.notices.findIndex((m) => m.read);
        this.notices.splice(i < 0 ? 0 : i, 1);
      }
    }
    this.emit(notice);
    return notice;
  }

  /** Simulates a click on an OS notification. */
  open(id: number): void {
    for (const h of this.openHandlers) h(id);
  }

  async list() {
    return this.snapshot();
  }

  async markRead(id: number) {
    const n = this.notices.find((m) => m.id === id);
    if (n) this.notices = this.notices.map((m) => (m.id === id ? { ...m, read: true } : m));
    this.emit(null);
    return !!n;
  }

  async markAllRead() {
    const n = this.notices.filter((m) => !m.read).length;
    this.notices = this.notices.map((m) => ({ ...m, read: true }));
    this.emit(null);
    return n;
  }

  async dismiss(id: number) {
    const before = this.notices.length;
    this.notices = this.notices.filter((m) => m.id !== id);
    this.emit(null);
    return this.notices.length !== before;
  }

  async record(): Promise<NoticesRecord> {
    return { version: 1, notices: this.notices.filter((n) => n.scope === 'project') };
  }

  async restore(record: unknown): Promise<RestoredNotices> {
    const r = record as NoticesRecord | null;
    let warning: string | null = null;
    const app = this.notices.filter((n) => n.scope === 'app');
    if (r && (r.version !== 1 || !Array.isArray(r.notices))) {
      warning = 'the saved notifications are damaged and were not loaded';
      this.notices = app;
    } else {
      const saved = (r?.notices ?? []) as Notice[];
      this.notices = [...app, ...saved.map((n) => ({ ...n, id: this.next++, scope: 'project' as const }))];
    }
    this.emit(null);
    return { snapshot: this.snapshot(), warning };
  }

  async onChange(handler: (e: NoticesEvent) => void): Promise<Unlisten> {
    this.handlers.add(handler);
    return () => this.handlers.delete(handler);
  }

  async onOpen(handler: (id: number) => void): Promise<Unlisten> {
    this.openHandlers.add(handler);
    return () => this.openHandlers.delete(handler);
  }
}

let instance: MockNotifications | null = null;

/** The one mock centre of this page (shared by the mock jobs, live alerts and the UI). */
export function mockNotifications(): MockNotifications {
  instance ??= new MockNotifications();
  if (typeof window !== 'undefined') (window as unknown as { __imtMockNotifications?: MockNotifications }).__imtMockNotifications = instance;
  return instance;
}

/** Forgets the shared instance (tests). */
export function resetMockNotifications(): void {
  instance = null;
}
