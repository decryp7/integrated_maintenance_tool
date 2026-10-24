import { TestBed } from '@angular/core/testing';
import { signal } from '@angular/core';
import { AppStatus } from '../core/tools.types';
import { LifecycleBackend } from '../core/tools-backend';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { NewLocalNotice, NotificationsStore } from '../notifications/notifications.store';
import { FakeDock, provideFakeDock } from '../../testing/helpers';
import { ChangelogRelease, decideWhatsNew, releaseFor } from './changelog';
import { CHANGELOG } from './changelog.generated';
import { WHATS_NEW_NOTICE, WHATS_NEW_SEEN_KEY, WHATS_NEW_VIEW, WhatsNewService } from './whats-new.service';
import { WHATS_NEW_VIEWS } from './provide-whats-new';
import { WhatsNewView } from './whats-new-view';

const NOTES: ChangelogRelease[] = [{ version: '2.0.0', date: '2026-01-01', groups: [{ title: 'Added', items: ['a'] }] }];

describe('whats new decisions', () => {
  it('finds the notes of a version', () => {
    expect(releaseFor(NOTES, '2.0.0')).toBe(NOTES[0]);
    expect(releaseFor(NOTES, '3.0.0')).toBeNull();
    expect(releaseFor(NOTES, null)).toBeNull();
    expect(releaseFor(NOTES, undefined)).toBeNull();
  });

  it('shows once after an update, only remembers on a first run or when there are no notes, and is silent otherwise', () => {
    expect(decideWhatsNew('1.0.0', '2.0.0', true)).toBe('show');
    expect(decideWhatsNew('1.0.0', '2.0.0', false)).toBe('remember');
    expect(decideWhatsNew(null, '2.0.0', true)).toBe('remember');
    expect(decideWhatsNew('2.0.0', '2.0.0', true)).toBe('nothing');
    expect(decideWhatsNew('1.0.0', null, true)).toBe('nothing');
    expect(decideWhatsNew('1.0.0', undefined, true)).toBe('nothing');
  });
});

describe("the What's new view declaration", () => {
  it('is a document of the dock with a declared layout (ADR 0056), reached from Help rather than the View menu', () => {
    const view = WHATS_NEW_VIEWS.find((v) => v.id === WHATS_NEW_VIEW)!;
    expect(view.layout?.area).toBe('document');
    expect(view.layout!.min.width).toBeLessThanOrEqual(view.layout!.pref.width);
    expect(view.layout!.priority).toBeGreaterThan(0);
    expect(view.menu).toBe('palette');
    expect(view.autoOpen).toBeUndefined();
  });
});

let engineVersion: string | null = '';

class FakeNotices {
  readonly posted: NewLocalNotice[] = [];
  readonly withdrawn: string[] = [];
  postLocal = (n: NewLocalNotice) => void this.posted.push(n);
  withdrawLocal = (key: string) => void this.withdrawn.push(key);
}

describe('WhatsNewService', () => {
  let dock: FakeDock;
  let notices: FakeNotices;
  beforeEach(() => {
    localStorage.clear();
    engineVersion = CHANGELOG[0]!.version;
    dock = new FakeDock();
    notices = new FakeNotices();
    TestBed.configureTestingModule({
      providers: [
        provideFakeDock(dock),
        { provide: NotificationsStore, useValue: notices },
        { provide: LifecycleService, useValue: { status: signal<AppStatus | null>(null) } },
        // The engine's answer, which the view reads (a popped-out view has no LifecycleService of its own that was ever started).
        {
          provide: LifecycleBackend,
          useValue: {
            appStatus: async () => {
              if (engineVersion === null) throw new Error('engine gone');
              return { version: engineVersion, safeMode: false, previousCrash: false, sessionAvailable: false };
            },
          },
        },
      ],
    });
  });
  afterEach(() => vi.restoreAllMocks());

  function setup(version: string | null) {
    const life = TestBed.inject(LifecycleService);
    life.status.set(version ? { version, safeMode: false, previousCrash: false, sessionAvailable: false } : null);
    return TestBed.inject(WhatsNewService);
  }
  const version = CHANGELOG[0]!.version;
  const shows = () => dock.calls.filter((c) => c.startsWith(`show:${WHATS_NEW_VIEW}`));

  it('remembers the version on a first run without opening anything or posting a notification', async () => {
    const svc = setup(version);
    await svc.checkOnStart();
    TestBed.tick();
    expect(shows()).toEqual([]);
    expect(notices.posted).toEqual([]);
    expect(localStorage.getItem(WHATS_NEW_SEEN_KEY)).toBe(version);
  });

  it('after an update opens the view once in the background and posts one calm notification that focuses it', async () => {
    localStorage.setItem(WHATS_NEW_SEEN_KEY, '0.0.1');
    const svc = setup(version);
    await svc.checkOnStart();
    TestBed.tick();
    expect(shows()).toEqual([`show:${WHATS_NEW_VIEW}:inactive`]);
    expect(localStorage.getItem(WHATS_NEW_SEEN_KEY)).toBe(version);
    expect(notices.posted).toHaveLength(1);
    const n = notices.posted[0]!;
    expect(n.key).toBe(WHATS_NEW_NOTICE);
    expect(n.severity).toBe('info');
    expect(n.source).toBe('update');
    expect(n.title).toBe(`Updated to ${version}`);
    expect(n.actionLabel).toBe("See what's new");
    n.run();
    expect(shows()).toEqual([`show:${WHATS_NEW_VIEW}:inactive`, `show:${WHATS_NEW_VIEW}`]);

    // A later start of the same version opens nothing and posts nothing.
    await svc.checkOnStart();
    TestBed.tick();
    expect(shows()).toHaveLength(2);
    expect(notices.posted).toHaveLength(1);
  });

  it('waits for the dock to have its layout before opening the view', async () => {
    localStorage.setItem(WHATS_NEW_SEEN_KEY, '0.0.1');
    dock.ready.set(false);
    const svc = setup(version);
    await svc.checkOnStart();
    TestBed.tick();
    expect(shows()).toEqual([]);
    expect(notices.posted).toEqual([]);
    dock.ready.set(true);
    TestBed.tick();
    expect(shows()).toEqual([`show:${WHATS_NEW_VIEW}:inactive`]);
    expect(notices.posted).toHaveLength(1);
    dock.ready.set(false);
    TestBed.tick();
    dock.ready.set(true);
    TestBed.tick();
    expect(shows()).toHaveLength(1);
  });

  it("Help › What's new opens and focuses the view and withdraws the notification", () => {
    const svc = setup(version);
    svc.open();
    expect(shows()).toEqual([`show:${WHATS_NEW_VIEW}`]);
    expect(notices.withdrawn).toEqual([WHATS_NEW_NOTICE]);
  });

  it('does nothing without a known version, and survives unavailable storage', async () => {
    const svc = setup(null);
    await svc.checkOnStart();
    TestBed.tick();
    expect(shows()).toEqual([]);
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    TestBed.inject(LifecycleService).status.set({ version, safeMode: false, previousCrash: false, sessionAvailable: false });
    await expect(svc.checkOnStart()).resolves.toBeUndefined();
  });

  async function viewFixture() {
    const fixture = TestBed.createComponent(WhatsNewView);
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  }
  const current = (el: HTMLElement) => el.querySelector('[data-testid=whats-new-current]')?.textContent ?? null;

  it('shows the running version, its groups and the earlier releases', async () => {
    const el = await viewFixture();
    expect(current(el)).toContain(`Version ${version}`);
    expect(el.querySelectorAll('[data-testid=whats-new-current] li').length).toBeGreaterThan(0);
    expect(el.querySelector('[data-testid=whats-new-earlier]')).not.toBeNull();
  });

  it('shows the notes of the version the engine reports, not the newest release', async () => {
    const older = CHANGELOG[1]!.version;
    engineVersion = older;
    const el = await viewFixture();
    expect(current(el)).toContain(`Version ${older}`);
    expect(current(el)).not.toContain(`Version ${CHANGELOG[0]!.version}`);
  });

  it('never shows the newest release in place of a version without notes', async () => {
    engineVersion = '9.9.9';
    const el = await viewFixture();
    expect(current(el)).toBeNull();
    expect(el.querySelector('[data-testid=whats-new-none]')).not.toBeNull();
  });

  it('never shows the newest release when the engine cannot say which version runs', async () => {
    engineVersion = null;
    const el = await viewFixture();
    expect(current(el)).toBeNull();
    expect(el.querySelector('[data-testid=whats-new-none]')).not.toBeNull();
  });
});
