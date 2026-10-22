import { TestBed } from '@angular/core/testing';
import { signal } from '@angular/core';
import { AppStatus } from '../core/tools.types';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { DIALOG_REF, DialogService } from '../ui/dialog.service';
import { ChangelogRelease, decideWhatsNew, releaseFor } from './changelog';
import { CHANGELOG } from './changelog.generated';
import { WHATS_NEW_SEEN_KEY, WhatsNewDialog, WhatsNewService } from './whats-new';

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

function dialogFixture() {
  const fixture = TestBed.createComponent(WhatsNewDialog);
  fixture.detectChanges();
  return fixture;
}

describe('WhatsNewService', () => {
  beforeEach(() => {
    localStorage.clear();
    TestBed.configureTestingModule({ providers: [
        { provide: DIALOG_REF, useValue: { close: () => undefined, closed: Promise.resolve(undefined) } },
        { provide: LifecycleService, useValue: { status: signal<AppStatus | null>(null) } },
      ],
    });
  });
  afterEach(() => vi.restoreAllMocks());

  function setup(version: string | null) {
    const life = TestBed.inject(LifecycleService);
    life.status.set(version ? { version, safeMode: false, previousCrash: false, sessionAvailable: false } : null);
    return { svc: TestBed.inject(WhatsNewService), dialogs: TestBed.inject(DialogService) };
  }
  const open = (d: DialogService) => d.stack().some((x) => x.component === WhatsNewDialog);
  const version = CHANGELOG[0]!.version;

  it('remembers the version on a first run without opening anything', () => {
    const { svc, dialogs } = setup(version);
    svc.checkOnStart();
    expect(open(dialogs)).toBe(false);
    expect(localStorage.getItem(WHATS_NEW_SEEN_KEY)).toBe(version);
  });

  it('opens once after an update, then stays closed for the same version', () => {
    localStorage.setItem(WHATS_NEW_SEEN_KEY, '0.0.1');
    const { svc, dialogs } = setup(version);
    svc.checkOnStart();
    expect(open(dialogs)).toBe(true);
    expect(localStorage.getItem(WHATS_NEW_SEEN_KEY)).toBe(version);
    dialogs.dismissTop();
    svc.checkOnStart();
    expect(open(dialogs)).toBe(false);
  });

  it('does nothing without a known version, and survives unavailable storage', () => {
    const { svc, dialogs } = setup(null);
    svc.checkOnStart();
    expect(open(dialogs)).toBe(false);
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    TestBed.inject(LifecycleService).status.set({ version, safeMode: false, previousCrash: false, sessionAvailable: false });
    expect(() => svc.checkOnStart()).not.toThrow();
  });

  it('opens on demand once, and the dialog shows the version, its groups and the earlier releases', async () => {
    const { svc, dialogs } = setup(version);
    svc.open();
    svc.open();
    expect(dialogs.stack().filter((d) => d.component === WhatsNewDialog)).toHaveLength(1);
    dialogs.dismissTop();
    const fixture = dialogFixture();
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelector('[data-testid=whats-new-current]')?.textContent).toContain(`Version ${version}`);
    expect(el.querySelectorAll('[data-testid=whats-new-current] li').length).toBeGreaterThan(0);
  });

  it('falls back to the newest release for a version without notes', () => {
    const { svc } = setup('9.9.9');
    expect(svc).toBeTruthy();
    const fixture = dialogFixture();
    expect((fixture.nativeElement as HTMLElement).querySelector('[data-testid=whats-new-current]')?.textContent).toContain(`Version ${CHANGELOG[0]!.version}`);
  });
});
