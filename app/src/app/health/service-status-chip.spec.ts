import { Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { all, el, settle, text } from '../../testing/helpers';
import { MockServiceHealth } from '../core/service-health/mock-service-health.backend';
import { OPEN_SETTINGS, RAISE_CAP, SERVICE_HEALTH_BACKEND } from './service-health.backend';
import { ServiceHealthStore } from './service-health.store';
import { ServiceStatusChip, shortName } from './service-status-chip';
import { WaitingFor } from './waiting-for';

const stopped = { message: "Debrief's log store stopped unexpectedly.", hint: 'Choose Try again.', detail: 'the store worker exited (signal: 6)' };

async function setup() {
  const mock = new MockServiceHealth();
  TestBed.configureTestingModule({ providers: [{ provide: SERVICE_HEALTH_BACKEND, useValue: mock }] });
  const fixture = TestBed.createComponent(ServiceStatusChip);
  const store = TestBed.inject(ServiceHealthStore);
  await settle(fixture);
  return { fixture, mock, store };
}

describe('ServiceStatusChip', () => {
  it('is absent while everything works', async () => {
    const { fixture } = await setup();
    expect(fixture.nativeElement.querySelector('[data-testid=service-health-chip]')).toBeNull();
  });

  it('shows one chip for a failure, with the plain message, the next step, folded details and Repair', async () => {
    const { fixture, mock } = await setup();
    mock.fail('store', stopped);
    mock.fail('store', stopped);
    await settle(fixture);
    expect(all(fixture, '[data-testid=service-health-chip]')).toHaveLength(1);
    expect(text(fixture)).toContain('Log store: stopped');
    el<HTMLButtonElement>(fixture, '[data-testid=service-health-chip]').click();
    await settle(fixture);
    const pop = document.querySelector('[data-testid=service-health-details]') as HTMLElement;
    expect(pop.textContent).toContain("Debrief's log store stopped unexpectedly.");
    expect(pop.textContent).toContain('Choose Try again.');
    expect(pop.querySelector('details')?.hasAttribute('open')).toBe(false);
    expect(pop.querySelector('code')?.textContent).toContain('signal: 6');
    (pop.querySelector('[data-testid=service-health-repair-store]') as HTMLButtonElement).click();
    await settle(fixture);
    expect(mock.repairs).toEqual(['store']);
    expect(fixture.nativeElement.querySelector('[data-testid=service-health-chip]')).toBeNull();
  });

  it('several outages are one chip with a count, each with its own repair', async () => {
    const { fixture, mock } = await setup();
    mock.fail('store', stopped);
    mock.fail('ai', { message: 'The AI worker was switched off.', hint: null, detail: null }, 'reset', 'quarantined');
    await settle(fixture);
    expect(text(fixture)).toContain('2 services need attention');
    el<HTMLButtonElement>(fixture, '[data-testid=service-health-chip]').click();
    await settle(fixture);
    expect(document.querySelectorAll('[data-testid^=service-health-repair-]')).toHaveLength(2);
    expect(document.querySelector('[data-testid=service-health-repair-ai]')?.textContent?.trim()).toBe('Try again');
  });

  it('a repair in progress says so instead of offering the button', async () => {
    const { fixture, mock } = await setup();
    mock.fail('store', stopped, 'restart', 'repairing');
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[data-testid=service-health-chip]').click();
    await settle(fixture);
    expect(document.querySelector('[data-testid=service-health-repair-store]')).toBeNull();
    expect(document.body.textContent).toContain('Repairing…');
  });

  it('opens by itself when a "Waiting for" link asks', async () => {
    const { fixture, mock, store } = await setup();
    mock.fail('store', stopped);
    await settle(fixture);
    expect(document.querySelector('[data-testid=service-health-details]')).toBeNull();
    store.showDetails();
    await settle(fixture);
    expect(document.querySelector('[data-testid=service-health-details]')).not.toBeNull();
  });

  it('names a service without its article', () => {
    expect(shortName('the log store')).toBe('Log store');
    expect(shortName('plugins')).toBe('Plugins');
  });
});

describe('WaitingFor', () => {
  @Component({ selector: 'debrief-host', imports: [WaitingFor], template: '<debrief-waiting-for service="store" />' })
  class Host {}

  it('is calm: names the service, no error colour, no Retry, and links to the chip', async () => {
    const mock = new MockServiceHealth();
    TestBed.configureTestingModule({ providers: [{ provide: SERVICE_HEALTH_BACKEND, useValue: mock }] });
    const f = TestBed.createComponent(Host);
    const store = TestBed.inject(ServiceHealthStore);
    mock.fail('store', stopped);
    await settle(f);
    expect(text(f)).toContain('Waiting for the log store');
    expect(f.nativeElement.querySelector('[data-testid=waiting-for-store]').getAttribute('role')).toBe('status');
    expect(text(f)).not.toContain('Retry');
    el<HTMLButtonElement>(f, '[data-testid=waiting-for-details]').click();
    expect(store.detailsRequested()).toBe(1);
  });
});

describe('ServiceStatusChip with a memory cap stop', () => {
  async function capSetup(raise: ((cap: unknown) => void) | null) {
    const mock = new MockServiceHealth();
    TestBed.configureTestingModule({
      providers: [{ provide: SERVICE_HEALTH_BACKEND, useValue: mock }, ...(raise ? [{ provide: RAISE_CAP, useValue: raise }] : [])],
    });
    const fixture = TestBed.createComponent(ServiceStatusChip);
    await settle(fixture);
    mock.fail('store', { message: 'The database worker was stopped: it used more than its 2048 MB memory cap', hint: 'Raise the cap or try again.', detail: null, raiseCap: { target: 'storeWorker', suggestedMb: 3328 } }, 'retry');
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[data-testid=service-health-chip]').click();
    await settle(fixture);
    return { fixture, mock };
  }

  it('offers Raise the cap… with the suggestion next to Try again', async () => {
    const raise = vi.fn();
    const { mock } = await capSetup(raise);
    expect(document.querySelector('[data-testid=service-health-repair-store]')?.textContent?.trim()).toBe('Try again');
    (document.querySelector('[data-testid=service-health-raise-cap-store]') as HTMLButtonElement).click();
    expect(raise).toHaveBeenCalledWith({ target: 'storeWorker', suggestedMb: 3328 });
    (document.querySelector('[data-testid=service-health-repair-store]') as HTMLButtonElement).click();
    expect(mock.repairs).toEqual(['store']);
  });

  it('offers no Raise the cap… where the app cannot open the setting', async () => {
    await capSetup(null);
    expect(document.querySelector('[data-testid=service-health-raise-cap-store]')).toBeNull();
  });
});

describe('ServiceStatusChip with a repair in Settings', () => {
  async function settingsSetup(open: ((section: string) => void) | null) {
    const mock = new MockServiceHealth();
    TestBed.configureTestingModule({
      providers: [{ provide: SERVICE_HEALTH_BACKEND, useValue: mock }, ...(open ? [{ provide: OPEN_SETTINGS, useValue: open }] : [])],
    });
    const fixture = TestBed.createComponent(ServiceStatusChip);
    await settle(fixture);
    mock.fail('browser', { message: 'The research browser could not be started.', hint: 'Choose another browser in Settings ▸ Web browser.', detail: null, settings: 'browser' }, 'openSettings');
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[data-testid=service-health-chip]').click();
    await settle(fixture);
    return { fixture, mock };
  }

  it('opens Settings at the section instead of asking the shell to repair', async () => {
    const open = vi.fn();
    const { mock } = await settingsSetup(open);
    const button = document.querySelector('[data-testid=service-health-repair-browser]') as HTMLButtonElement;
    expect(button.textContent?.trim()).toBe('Open Settings…');
    button.click();
    expect(open).toHaveBeenCalledWith('browser');
    expect(mock.repairs).toEqual([]);
  });

  it('offers no button where the app cannot open Settings', async () => {
    await settingsSetup(null);
    expect(document.querySelector('[data-testid=service-health-repair-browser]')).toBeNull();
  });
});
