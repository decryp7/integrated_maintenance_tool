import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { all, buttonByText, el, settle, text } from '../../testing/helpers';
import { MAX_VISIBLE, HostNotification, NOTIFICATION_SOURCES, NotificationSource, persists, visibleNotifications } from './notification-host.model';
import { NotificationHost } from './notification-host';

const n = (cause: string, extra: Partial<HostNotification> = {}): HostNotification => ({ cause, severity: 'info', title: cause, dismiss: () => undefined, ...extra });

describe('visibleNotifications (the one display rule)', () => {
  it('one toast per cause: the same cause three times is one, in the newest position', () => {
    const out = visibleNotifications([n('a', { title: 'first' }), n('b'), n('a', { title: 'third', count: 3 })]);
    expect(out.map((x) => [x.cause, x.title])).toEqual([
      ['b', 'b'],
      ['a', 'third'],
    ]);
  });

  it('never toasts a cause the status chip states', () => {
    expect(visibleNotifications([n('health:store'), n('job:1')]).map((x) => x.cause)).toEqual(['job:1']);
  });

  it('shows at most the newest few', () => {
    const many = ['a', 'b', 'c', 'd', 'e'].map((c) => n(c));
    expect(visibleNotifications(many).map((x) => x.cause)).toEqual(['c', 'd', 'e']);
    expect(MAX_VISIBLE).toBe(3);
  });

  it('errors and critical failures persist, the rest time out', () => {
    expect((['info', 'success', 'warning', 'error', 'critical'] as const).map(persists)).toEqual([false, false, false, true, true]);
  });
});

describe('NotificationHost', () => {
  function setup(items: HostNotification[]) {
    const list = signal<readonly HostNotification[]>(items);
    const source: NotificationSource = { items: list };
    TestBed.configureTestingModule({ providers: [{ provide: NOTIFICATION_SOURCES, multi: true, useValue: source }] });
    const f = TestBed.createComponent(NotificationHost);
    return { f, list };
  }

  it('shows the title, the body, a count for a repeated cause, and updates in place', async () => {
    const { f, list } = setup([n('job:1', { title: 'Export failed', body: 'Disk full', count: 1 })]);
    await settle(f);
    expect(text(f)).toContain('Export failed');
    expect(text(f)).toContain('Disk full');
    expect(f.nativeElement.querySelector('[data-testid=notification-count]')).toBeNull();
    list.set([n('job:1', { title: 'Export failed', body: 'Disk full', count: 3 })]);
    await settle(f);
    expect(all(f, '[data-testid=notification]')).toHaveLength(1);
    expect(el(f, '[data-testid=notification-count]').textContent).toContain('×3');
  });

  it('collapses technical details behind Details', async () => {
    const { f } = setup([n('a', { details: 'stack: boom' })]);
    await settle(f);
    expect(f.nativeElement.querySelector('[data-testid=notification-details]')).toBeNull();
    const toggle = el<HTMLButtonElement>(f, '[data-testid=notification-details-toggle]');
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    toggle.click();
    await settle(f);
    expect(el(f, '[data-testid=notification-details]').textContent).toContain('stack: boom');
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
  });

  it('errors are alerts, others status; severity is on the element', async () => {
    const { f } = setup([n('e', { severity: 'error' }), n('i')]);
    await settle(f);
    const toasts = all(f, '[data-testid=notification]');
    expect(toasts.map((t) => [t.getAttribute('role'), t.getAttribute('data-severity')])).toEqual([
      ['alert', 'error'],
      ['status', 'info'],
    ]);
  });

  it('runs an action; a failing action keeps the toast and says why', async () => {
    const ok = vi.fn();
    const { f } = setup([n('a', { actions: [{ label: 'Do it', run: ok }] }), n('b', { actions: [{ label: 'Break', run: () => Promise.reject(new Error('nope')) }] })]);
    await settle(f);
    buttonByText(f, 'Do it').click();
    expect(ok).toHaveBeenCalled();
    buttonByText(f, 'Break').click();
    await settle(f);
    expect(text(f)).toContain('nope');
  });

  it('dismiss, hold and release go to the source', async () => {
    const dismiss = vi.fn();
    const hold = vi.fn();
    const release = vi.fn();
    const { f } = setup([n('a', { dismiss, hold, release })]);
    await settle(f);
    const toast = el(f, '[data-testid=notification]');
    toast.dispatchEvent(new MouseEvent('mouseenter'));
    toast.dispatchEvent(new MouseEvent('mouseleave'));
    // Dismissing is in the spec below; here only the pointer.
    el<HTMLButtonElement>(f, '[data-testid=notification-dismiss]').click();
    expect([hold.mock.calls.length, release.mock.calls.length, dismiss.mock.calls.length]).toEqual([1, 1, 1]);
  });

  it('merges several sources by cause and hides health causes', async () => {
    const a: NotificationSource = { items: signal([n('x', { title: 'from a' })]) };
    const b: NotificationSource = { items: signal([n('x', { title: 'from b' }), n('health:store')]) };
    TestBed.configureTestingModule({
      providers: [
        { provide: NOTIFICATION_SOURCES, multi: true, useValue: a },
        { provide: NOTIFICATION_SOURCES, multi: true, useValue: b },
      ],
    });
    const f = TestBed.createComponent(NotificationHost);
    await settle(f);
    expect(all(f, '[data-testid=notification]')).toHaveLength(1);
    expect(text(f)).toContain('from b');
  });

  it('renders nothing without sources', async () => {
    TestBed.configureTestingModule({});
    const f = TestBed.createComponent(NotificationHost);
    await settle(f);
    expect(all(f, '[data-testid=notification]')).toHaveLength(0);
  });
});
