import { Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { all, buttonByText, el, settle, text } from '../../testing/helpers';
import { MockServiceHealth } from '../core/service-health/mock-service-health.backend';
import { SERVICE_HEALTH_BACKEND } from '../health/service-health.backend';
import { ErrorDetails } from './error-details';

const stopped = { message: "Debrief's log store stopped unexpectedly.", hint: 'Choose Try again.', detail: null };

@Component({ selector: 'debrief-host', imports: [ErrorDetails], template: '<debrief-error-details summary="Could not load events." detail="boom" (retry)="retried()" />' })
class Host {
  retried = vi.fn();
}

async function setup() {
  const mock = new MockServiceHealth();
  TestBed.configureTestingModule({ providers: [{ provide: SERVICE_HEALTH_BACKEND, useValue: mock }] });
  const f = TestBed.createComponent(Host);
  await settle(f);
  return { f, mock };
}

describe('ErrorDetails with service health', () => {
  it('an error of the request itself shows the sentence, Retry and folded technical details', async () => {
    const { f } = await setup();
    expect(text(f)).toContain('Could not load events.');
    buttonByText(f, 'Retry').click();
    expect(f.componentInstance.retried).toHaveBeenCalledTimes(1);
    expect(el(f, 'details').hasAttribute('open')).toBe(false);
  });

  it('while the log store is not working the panel waits calmly: no error text, no Retry of its own', async () => {
    const { f, mock } = await setup();
    mock.fail('store', stopped);
    await settle(f);
    expect(text(f)).toContain('Waiting for the log store');
    expect(text(f)).not.toContain('Could not load events.');
    expect(all(f, 'button').map((b) => b.textContent?.trim())).toEqual(['See what happened']);
  });

  it('loads again by itself when the store comes back, once', async () => {
    const { f, mock } = await setup();
    mock.fail('store', stopped);
    await settle(f);
    expect(f.componentInstance.retried).not.toHaveBeenCalled();
    mock.recover('store');
    await settle(f);
    expect(f.componentInstance.retried).toHaveBeenCalledTimes(1);
    expect(text(f)).toContain('Could not load events.');
    mock.fail('store', stopped);
    await settle(f);
    mock.recover('store');
    await settle(f);
    expect(f.componentInstance.retried).toHaveBeenCalledTimes(2);
  });

  it('another service being down does not hide an error that belongs to the request', async () => {
    const { f, mock } = await setup();
    mock.fail('ai', { message: 'The AI worker stopped.', hint: null, detail: null });
    await settle(f);
    expect(text(f)).toContain('Could not load events.');
  });
});
