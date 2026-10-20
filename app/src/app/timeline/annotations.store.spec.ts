import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { AnnotationDto } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { provideTinyBackend, settle, tinyBackend, useGate } from '../../testing/helpers';
import { TimelineAnnotations } from './annotations.store';
import { at } from '../../testing/must';

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => ((resolve = res), (reject = rej)));
  return { promise, resolve, reject };
}

describe('TimelineAnnotations', () => {
  it('ignores responses to superseded requests and clears when nothing is open', async () => {
    const backend = tinyBackend();
    const calls: ReturnType<typeof deferred<AnnotationDto[]>>[] = [];
    vi.spyOn(backend, 'annotations').mockImplementation(() => {
      const d = deferred<AnnotationDto[]>();
      calls.push(d);
      return d.promise;
    });
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const store = TestBed.inject(WorkspaceStore);
    const ann = TestBed.inject(TimelineAnnotations);
    useGate(ann.gate);
    await store.refreshSummary();
    store.setBuckets(100);
    await settle(undefined, 8);
    expect(calls.length).toBeGreaterThan(0);
    const first = at(calls, calls.length - 1);
    await store.refreshSummary(); // new data version: a new request supersedes the first
    await settle(undefined, 8);
    const second = at(calls, calls.length - 1);
    expect(second).not.toBe(first);
    second.resolve([]);
    first.resolve([{ kind: 'stale' } as AnnotationDto]);
    first.reject?.('ignored');
    await settle(undefined, 4);
    expect(ann.items()).toEqual([]);

    // A later failure of a superseded request is ignored too.
    await store.refreshSummary();
    await settle(undefined, 8);
    const third = at(calls, calls.length - 1);
    await store.refreshSummary();
    await settle(undefined, 8);
    at(calls, calls.length - 1).resolve(null as unknown as AnnotationDto[]);
    third.reject('late failure');
    await settle(undefined, 4);
    expect(ann.error()).toBeNull();
    expect(ann.items()).toEqual([]);

    await store.clearWorkspace();
    await settle(undefined, 8);
    expect(ann.items()).toEqual([]);
    expect(ann.isShown('x')).toBe(true);
    ann.toggle('x');
    expect(ann.isShown('x')).toBe(false);
  });

  it('fetches only while a view showing the annotations is visible', async () => {
    const backend = tinyBackend();
    const spy = vi.spyOn(backend, 'annotations');
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const store = TestBed.inject(WorkspaceStore);
    const ann = TestBed.inject(TimelineAnnotations);
    await store.refreshSummary();
    store.setBuckets(100);
    await settle(undefined, 8);
    expect(spy).not.toHaveBeenCalled();
    const timeline = useGate(ann.gate, signal(false));
    await settle(undefined, 8);
    expect(spy).not.toHaveBeenCalled();
    const runsView = useGate(ann.gate);
    await settle(undefined, 8);
    expect(spy).toHaveBeenCalledTimes(1);
    runsView.release();
    await store.refreshSummary();
    await settle(undefined, 8);
    expect(spy).toHaveBeenCalledTimes(1);
    timeline.visible.set(true);
    await settle(undefined, 8);
    expect(spy).toHaveBeenCalledTimes(2);
  });
});
