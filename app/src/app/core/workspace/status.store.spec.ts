import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { NOTICE_MS, WorkspaceStatusStore } from './status.store';

describe('WorkspaceStatusStore', () => {
  afterEach(() => vi.useRealTimers());

  // Regression: a confirmation ("Exported 120 events") stayed until it was closed by hand.
  it('a notice goes after NOTICE_MS, held while hovered; an error stays', () => {
    vi.useFakeTimers();
    const store = TestBed.inject(WorkspaceStatusStore);
    store.setNotice('Exported 120 events');
    store.setError('Disk full');
    vi.advanceTimersByTime(NOTICE_MS - 1);
    expect(store.notice()).toBe('Exported 120 events');
    vi.advanceTimersByTime(1);
    expect(store.notice()).toBeNull();
    expect(store.error()).toBe('Disk full');

    store.setNotice('Saved');
    store.holdNotice();
    vi.advanceTimersByTime(NOTICE_MS * 3);
    expect(store.notice()).toBe('Saved');
    store.releaseNotice();
    vi.advanceTimersByTime(NOTICE_MS);
    expect(store.notice()).toBeNull();

    // A new notice restarts the time; clearing one stops its timer.
    store.setNotice('First');
    vi.advanceTimersByTime(NOTICE_MS - 100);
    store.setNotice('Second');
    vi.advanceTimersByTime(200);
    expect(store.notice()).toBe('Second');
    store.setNotice(null);
    store.releaseNotice(); // nothing to restart
    vi.advanceTimersByTime(NOTICE_MS);
    expect(store.notice()).toBeNull();
  });
});
