import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { settle } from '../../testing/helpers';
import { MockServiceHealth } from '../core/service-health/mock-service-health.backend';
import { SERVICE_HEALTH_BACKEND, ServiceHealthBackend } from './service-health.backend';
import { ServiceHealthStore } from './service-health.store';
import { healthCause, isHealthCause, isServing, repairLabel } from './service-health.types';

const cause = { message: "Debrief's log store stopped unexpectedly.", hint: 'Choose Try again.', detail: 'signal: 6', raiseCap: null };

function setup(backend: ServiceHealthBackend = new MockServiceHealth()) {
  TestBed.configureTestingModule({ providers: [{ provide: SERVICE_HEALTH_BACKEND, useValue: backend }] });
  const store = TestBed.inject(ServiceHealthStore);
  return { store, backend };
}

describe('ServiceHealthStore', () => {
  it('claims nothing is healthy until the first snapshot arrived (a new window must not assume it)', async () => {
    let release: (() => void) | undefined;
    const slow = new MockServiceHealth();
    const snapshot = slow.snapshot.bind(slow);
    slow.snapshot = () => new Promise((r) => (release = () => r(snapshot())));
    const { store } = setup(slow);
    expect(store.isReady()).toBe(false);
    expect(store.isServing('store')).toBe(false);
    release?.();
    await settle();
    expect(store.isReady()).toBe(true);
    expect(store.isServing('store')).toBe(true);
    expect(store.isLoaded()).toBe(true);
  });

  it('follows the engine: a failure is one problem, recovery clears it', async () => {
    const { store, backend } = setup();
    await settle();
    const mock = backend as MockServiceHealth;
    expect(store.problems()).toEqual([]);
    mock.fail('store', cause);
    mock.fail('store', cause);
    expect(store.problems().map((p) => [p.service, p.state, p.repair])).toEqual([['store', 'down', 'restart']]);
    expect(store.isServing('store')).toBe(false);
    expect(store.isServing('ai')).toBe(true);
    expect(store.health('store')?.cause?.detail).toBe('signal: 6');
    mock.recover('store');
    expect(store.problems()).toEqual([]);
  });

  it('keeps the newest snapshot when events cross', async () => {
    const { store } = setup();
    await settle();
    store.apply({ revision: 9, services: [{ service: 'ai', label: 'the AI worker', state: 'down', cause, repair: 'reset' }] });
    store.apply({ revision: 3, services: [] });
    expect(store.problems().map((p) => p.service)).toEqual(['ai']);
  });

  it('a service the engine does not list has not failed, once the window has heard', async () => {
    const { store } = setup();
    await settle();
    expect(store.isServing('nope')).toBe(true);
    expect(store.health('nope')).toBeNull();
  });

  it('Repair asks the shell and takes the answer; a refused repair is an error status, not a throw', async () => {
    const { store, backend } = setup();
    await settle();
    const mock = backend as MockServiceHealth;
    mock.fail('store', cause);
    await store.repair('store');
    expect(mock.repairs).toEqual(['store']);
    expect(store.problems()).toEqual([]);
    mock.repair = async () => {
      throw new Error('there is nothing to repair');
    };
    await store.repair('store');
    expect(store.error()).toBe('there is nothing to repair');
  });

  it('a failed first read is an error status that Retry (load) can clear', async () => {
    const bad = new MockServiceHealth();
    const original = bad.snapshot.bind(bad);
    bad.snapshot = vi.fn(async () => {
      throw new Error('no answer');
    });
    const { store } = setup(bad);
    await settle();
    expect(store.error()).toBe('no answer');
    bad.snapshot = original;
    await store.load();
    expect(store.isLoaded()).toBe(true);
  });

  it('stops listening when it is destroyed', async () => {
    const mock = new MockServiceHealth();
    const { store } = setup(mock);
    await settle();
    TestBed.resetTestingModule();
    mock.fail('store', cause);
    expect(store.problems()).toEqual([]);
  });

  it('showDetails counts requests so the chip can open', async () => {
    const { store } = setup();
    expect(store.detailsRequested()).toBe(0);
    store.showDetails();
    store.showDetails();
    expect(store.detailsRequested()).toBe(2);
  });
});

describe('service health types', () => {
  it('names the cause key like the engine and recognises it', () => {
    expect(healthCause('store')).toBe('health:store');
    expect(isHealthCause('health:ai')).toBe(true);
    expect(isHealthCause('job:1')).toBe(false);
  });

  it('only ok and starting serve', () => {
    expect((['ok', 'starting', 'degraded', 'down', 'quarantined', 'repairing'] as const).map(isServing)).toEqual([true, true, false, false, false, false]);
  });

  it('every repair action has a button label', () => {
    for (const a of ['restart', 'reset', 'liftQuarantine', 'install', 'retry', 'useFallback'] as const) expect(repairLabel(a).length).toBeGreaterThan(0);
  });
});
