import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { WorkspaceStore } from '../core/workspace.store';
import { provideTinyBackend } from '../../testing/helpers';
import { WorkspaceSource } from './workspace-source';

describe('WorkspaceSource', () => {
  it('shows the error and the notice as one notification each, with stable causes', () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const store = TestBed.inject(WorkspaceStore);
    const source = TestBed.inject(WorkspaceSource);
    expect(source.items()).toEqual([]);
    store.setError('Something failed');
    store.setError('Something else failed');
    store.setNotice('Exported 3 events');
    expect(source.items().map((i) => [i.cause, i.severity, i.title])).toEqual([
      ['workspace.error', 'error', 'Something else failed'],
      ['workspace.notice', 'success', 'Exported 3 events'],
    ]);
    source.items()[0]?.dismiss();
    expect(source.items().map((i) => i.cause)).toEqual(['workspace.notice']);
  });

  it('offers the notice action and pauses the notice under the pointer', () => {
    vi.useFakeTimers();
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const store = TestBed.inject(WorkspaceStore);
    const source = TestBed.inject(WorkspaceSource);
    const run = vi.fn();
    store.setNotice('Saved Case 7', { label: 'Undo', run });
    const [item] = source.items();
    expect(item?.actions?.map((a) => a.label)).toEqual(['Undo']);
    item?.hold?.();
    vi.advanceTimersByTime(60_000);
    expect(source.items()).toHaveLength(1);
    item?.actions?.[0]?.run();
    expect(run).toHaveBeenCalled();
    expect(source.items()).toEqual([]);
    vi.useRealTimers();
  });
});
