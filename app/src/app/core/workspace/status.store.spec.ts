import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { WorkspaceStatusStore } from './status.store';

describe('WorkspaceStatusStore', () => {
  // The notification host times the notice out; the store only holds what was set until it is cleared.
  it('holds a notice and an error until they are cleared', () => {
    const store = TestBed.inject(WorkspaceStatusStore);
    store.setNotice('Exported 120 events');
    store.setError('Disk full');
    expect([store.notice(), store.error()]).toEqual(['Exported 120 events', 'Disk full']);
    store.setNotice(null);
    store.dismissError();
    expect([store.notice(), store.error(), store.noticeAction()]).toEqual([null, null, null]);
  });

  it('runs the notice action once and dismisses the notice', () => {
    const store = TestBed.inject(WorkspaceStatusStore);
    const run = vi.fn();
    store.setNotice('Saved Case 7', { label: 'Undo', run });
    expect(store.noticeAction()?.label).toBe('Undo');
    store.runNoticeAction();
    store.runNoticeAction();
    expect(run).toHaveBeenCalledTimes(1);
    expect(store.notice()).toBeNull();
  });
});
