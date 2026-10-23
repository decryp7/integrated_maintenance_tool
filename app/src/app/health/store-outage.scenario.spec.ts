import { Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { all, buttonByText, el, provideTinyBackend, settle, text } from '../../testing/helpers';
import { MockServiceHealth } from '../core/service-health/mock-service-health.backend';
import { MockNotifications } from '../notifications/mock-notifications';
import { NotificationsBackend } from '../notifications/notifications-backend';
import { NotificationsSource } from '../notifications/notifications-source';
import { NotificationsStore } from '../notifications/notifications.store';
import { NoticeActions } from '../notifications/notice-actions';
import { ErrorDetails } from '../ui/error-details';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { NotificationHost } from './notification-host';
import { provideNotificationSource } from './provide-notification-host';
import { SERVICE_HEALTH_BACKEND } from './service-health.backend';
import { ServiceStatusChip } from './service-status-chip';

@Component({
  selector: 'debrief-scenario',
  imports: [NotificationHost, ServiceStatusChip, ErrorDetails],
  template: `
    <debrief-service-status-chip />
    <debrief-notification-host />
    <debrief-error-details summary="Could not load events." (retry)="grid()" />
    <debrief-error-details summary="Could not draw the timeline." (retry)="timeline()" />
  `,
})
class Scenario {
  grid = vi.fn();
  timeline = vi.fn();
}

const cause = { message: "Debrief's log store stopped unexpectedly.", hint: 'Choose Try again.', detail: 'the store worker exited (signal: 6)' };

/** The engine, as the shell posts it: one `health:store` notification the first time, updated for each repeat. */
function engineNotice(centre: MockNotifications) {
  centre.post({ cause: 'health:store', source: 'health', severity: 'error', scope: 'app', title: cause.message, body: cause.hint, primary: { type: 'repairService', service: 'store' } });
}

async function setup() {
  const health = new MockServiceHealth();
  const centre = new MockNotifications();
  TestBed.configureTestingModule({
    providers: [
      provideTinyBackend(),
      { provide: SERVICE_HEALTH_BACKEND, useValue: health },
      { provide: NotificationsBackend, useValue: centre },
      { provide: NoticeActions, useValue: { run: async () => null } },
      { provide: COMMAND_CONTEXT, useValue: { showView: () => undefined } },
      provideNotificationSource(NotificationsSource),
    ],
  });
  const f = TestBed.createComponent(Scenario);
  TestBed.inject(NotificationsStore);
  await settle(f);
  return { f, health, centre };
}

describe('the log store worker dies (ADR 0055)', () => {
  it('is one chip, no toast, and every panel waits; Repair restarts it and the panels load again by themselves', async () => {
    const { f, health, centre } = await setup();
    // Three commands meet the same failure: the engine reports it once and the registry announces it once.
    for (let i = 0; i < 3; i++) {
      health.fail('store', cause);
      engineNotice(centre);
    }
    await settle(f);
    // One chip.
    expect(all(f, '[data-testid=service-health-chip]')).toHaveLength(1);
    // The centre holds one message for the cause (count 3), and the host shows no toast for a health cause.
    const notices = (await centre.list()).notices;
    expect(notices).toHaveLength(1);
    expect(notices[0]?.count).toBe(3);
    expect(all(f, '[data-testid=notice-toast]')).toHaveLength(0);
    expect(all(f, '[data-testid=notification]')).toHaveLength(0);
    // Both panels wait, calmly: no per-panel error.
    expect(all(f, '[data-testid=waiting-for-store]')).toHaveLength(2);
    expect(text(f)).not.toContain('Could not load events.');
    // Repair is served by the shell and restarts the store.
    el<HTMLButtonElement>(f, '[data-testid=service-health-chip]').click();
    await settle(f);
    (document.querySelector('[data-testid=service-health-repair-store]') as HTMLButtonElement).click();
    await settle(f);
    expect(health.repairs).toEqual(['store']);
    centre.resolve('health:store');
    await settle(f);
    expect(all(f, '[data-testid=service-health-chip]')).toHaveLength(0);
    expect((await centre.list()).notices).toHaveLength(0);
    // Each panel loads again by itself, once.
    expect(f.componentInstance.grid).toHaveBeenCalledTimes(1);
    expect(f.componentInstance.timeline).toHaveBeenCalledTimes(1);
    expect(buttonByText(f, 'Retry')).toBeTruthy();
  });

  it('a second window that starts while the store is down does not assume it is healthy', async () => {
    const health = new MockServiceHealth();
    health.fail('store', cause);
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), { provide: SERVICE_HEALTH_BACKEND, useValue: health }] });
    const f = TestBed.createComponent(Scenario);
    await settle(f);
    expect(all(f, '[data-testid=service-health-chip]')).toHaveLength(1);
    expect(all(f, '[data-testid=waiting-for-store]')).toHaveLength(2);
  });
});

describe('each other service fails the same way', () => {
  it.each([
    ['ai', 'The AI worker was switched off.', 'quarantined', 'reset', 'AI worker: switched off'],
    ['plugins', 'The plugin net.x was switched off after it failed several times.', 'quarantined', 'liftQuarantine', 'Plugins: switched off'],
    ['browser', 'The research browser is not available.', 'down', 'install', 'Research browser: stopped'],
    ['downloads', 'A download failed: Download Qwen.', 'down', 'retry', 'Downloads: stopped'],
  ] as const)('%s: one chip with the cause, its repair action, and the repair works', async (service, message, state, repair, chip) => {
    const { f, health } = await setup();
    health.fail(service, { message, hint: 'Do the next step.', detail: 'technical' }, repair, state);
    health.fail(service, { message, hint: 'Do the next step.', detail: 'technical' }, repair, state);
    await settle(f);
    expect(text(f)).toContain(chip);
    el<HTMLButtonElement>(f, '[data-testid=service-health-chip]').click();
    await settle(f);
    expect(document.querySelector(`[data-testid=service-health-${service}]`)?.textContent).toContain(message);
    (document.querySelector(`[data-testid=service-health-repair-${service}]`) as HTMLButtonElement).click();
    await settle(f);
    expect(health.repairs).toEqual([service]);
    expect(all(f, '[data-testid=service-health-chip]')).toHaveLength(0);
  });
});
