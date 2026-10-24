import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { WorkspaceStore } from '../core/workspace.store';
import { provideTinyBackend } from '../../testing/helpers';
import { DockService } from '../dock/dock.service';
import { NarrowWindow } from './narrow-window';

type Verdict = 'hide' | 'show' | 'keep';

function setup() {
  TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
  const dock = TestBed.inject(DockService);
  let verdict: Verdict = 'keep';
  const verdicts = vi.spyOn(dock, 'explorerVerdict').mockImplementation(() => verdict);
  const listeners: (() => void)[] = [];
  vi.spyOn(dock, 'onWindowPlan').mockImplementation((l) => {
    listeners.push(l);
    return () => undefined;
  });
  return { narrow: TestBed.inject(NarrowWindow), store: TestBed.inject(WorkspaceStore), set: (v: Verdict) => (verdict = v), listeners, verdicts };
}

describe('NarrowWindow (the Explorer follows the layout plan)', () => {
  it('closes the Explorer when the plan has no room for it and brings it back when there is', () => {
    const { narrow, store, set } = setup();
    expect(store.sidebarOpen()).toBe(true);
    set('hide');
    narrow.evaluate();
    expect(store.sidebarOpen()).toBe(false);
    set('keep');
    narrow.evaluate();
    expect(store.sidebarOpen()).toBe(false);
    set('show');
    narrow.evaluate();
    expect(store.sidebarOpen()).toBe(true);
  });

  it('does not reopen an Explorer the user had closed', () => {
    const { narrow, store, set } = setup();
    store.toggleSidebar();
    set('hide');
    narrow.evaluate();
    set('show');
    narrow.evaluate();
    expect(store.sidebarOpen()).toBe(false);
  });

  it('leaves an Explorer the user opened by hand while there is no room, until it fits again', () => {
    const { narrow, store, set } = setup();
    set('hide');
    narrow.evaluate();
    store.toggleSidebar();
    narrow.evaluate();
    expect(store.sidebarOpen()).toBe(true);
    narrow.evaluate();
    expect(store.sidebarOpen()).toBe(true);
    set('keep');
    narrow.evaluate();
    set('hide');
    narrow.evaluate();
    expect(store.sidebarOpen()).toBe(false);
  });

  it('re-evaluates after every dock re-plan, and starts once', () => {
    const { narrow, store, set, listeners, verdicts } = setup();
    narrow.start();
    narrow.start();
    expect(listeners).toHaveLength(1);
    set('hide');
    listeners[0]?.();
    expect(store.sidebarOpen()).toBe(false);
    expect(verdicts).toHaveBeenCalled();
  });
});
