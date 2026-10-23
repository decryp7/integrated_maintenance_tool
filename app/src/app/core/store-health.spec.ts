import { Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { el, provideTinyBackend, settle } from '../../testing/helpers';
import { PerformanceBackend } from '../settings/performance/performance-backend';
import { ErrorDetails } from '../ui/error-details';
import { WorkspaceToasts } from '../shell/workspace-toasts';
import { WorkspaceStore } from './workspace.store';
import { CmdError } from './cmd-error';
import { isStoreDown, reportCommandFailure, reportCommandSuccess, StoreHealth } from './store-health';

const DOWN = new CmdError({
  code: 'unavailable',
  message: "Debrief's log store couldn't start: the system refused to load a component that ships with Debrief.",
  retryable: true,
  hint: 'Install the latest Debrief over this copy.',
  detail: 'the store worker failed 3 times (signal: 6 (SIGABRT))',
});

@Component({ imports: [ErrorDetails], template: `<debrief-error-details summary="Couldn't load the timeline." detail="raw" (retry)="retried = retried + 1" />` })
class Host {
  retried = 0;
}

describe('StoreHealth', () => {
  beforeEach(() => TestBed.configureTestingModule({ providers: provideTinyBackend() }));

  it('recognises the engine\'s "log store is not running" error only', () => {
    expect(isStoreDown(DOWN)).toBe(true);
    expect(isStoreDown(new CmdError({ code: 'unavailable', message: 'video support is not ready yet', retryable: false }))).toBe(false);
    expect(isStoreDown(new Error("Debrief's log store"))).toBe(false);
  });

  it('is down after one failing command, once, and recovers when a store command succeeds', () => {
    const health = TestBed.inject(StoreHealth);
    expect(health.down()).toBeNull();
    reportCommandFailure(DOWN);
    reportCommandFailure(DOWN);
    expect(health.down()).toEqual({ message: DOWN.message, hint: DOWN.hint, detail: DOWN.detail });
    reportCommandSuccess('open_paths');
    expect(health.down()).not.toBeNull();
    reportCommandSuccess('query_page');
    expect(health.down()).toBeNull();
  });

  it('shows one calm state in a panel, with no technical text, and Try again restarts the worker before the panel retries', async () => {
    const restart = vi.spyOn(TestBed.inject(PerformanceBackend), 'retryStoreWorker').mockResolvedValue();
    const fixture = TestBed.createComponent(Host);
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('[data-testid=store-down-empty]')).toBeNull();
    reportCommandFailure(DOWN);
    await settle(fixture);
    expect(el(fixture, '[data-testid=store-down-empty]').textContent).toContain("isn't running");
    expect(fixture.nativeElement.textContent).not.toContain('SIGABRT');
    expect(fixture.nativeElement.querySelector('details')).toBeNull();
    el(fixture, '[data-testid=store-down-retry]').click();
    await settle(fixture);
    expect(restart).toHaveBeenCalledTimes(1);
    expect(fixture.componentInstance.retried).toBe(1);
    expect(TestBed.inject(StoreHealth).down()).toBeNull();
  });

  it('does not repeat the failure as a banner while the store is down', async () => {
    const ws = TestBed.inject(WorkspaceStore);
    const fixture = TestBed.createComponent(WorkspaceToasts);
    await settle(fixture);
    ws.setError(`${DOWN.message} ${DOWN.hint}`);
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('[role=alert]')).not.toBeNull();
    reportCommandFailure(DOWN);
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('[role=alert]')).toBeNull();
    ws.setError('Could not save the profile');
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('[role=alert]')).not.toBeNull();
  });
});
