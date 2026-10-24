import { TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';
import { provideTinyBackend } from '../../testing/helpers';
import { CmdError, isServiceOutage } from './cmd-error';
import { bannerText } from './workspace/workspace-data.store';
import { WorkspaceStore } from './workspace.store';

const outage = new CmdError({ code: 'unavailable', message: "Debrief's log store stopped unexpectedly.", retryable: true, cause: 'health:store' });

describe('a service outage in a rejected command', () => {
  it('carries the cause of the engine, and nothing else is one', () => {
    expect(outage.cause).toBe('health:store');
    expect(isServiceOutage(outage)).toBe(true);
    expect(isServiceOutage(new CmdError({ code: 'failed', message: 'x', retryable: false }))).toBe(false);
    expect(isServiceOutage(new Error('x'))).toBe(false);
  });

  it('is not repeated as a banner, but any other failure is', () => {
    expect(bannerText(outage)).toBeNull();
    expect(bannerText(new CmdError({ code: 'failed', message: 'Disk full', retryable: false }))).toBe('Disk full');
  });

  it('a banner for an outage is cleared instead of set (the status chip says it)', async () => {
    const store = TestBed.configureTestingModule({ providers: provideTinyBackend() }).inject(WorkspaceStore);
    store.setError('old');
    store.setError(bannerText(outage));
    expect(store.error()).toBeNull();
  });
});
