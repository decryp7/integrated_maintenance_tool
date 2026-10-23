import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, provideTinyBackendBare, settle, testLogger, text, tinyDataset } from '../../testing/helpers';
import { alert } from '../../testing/live-helpers';
import { LiveAlert } from '../core/api.types';
import { MockBackend } from '../core/mock/mock-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { DialogService } from '../ui/dialog.service';
import { TimeCursor } from '../analysis/time-cursor';
import { LiveTail } from '../workspace/live-tail';
import { NotificationHost } from '../health/notification-host';
import { provideNotificationSource } from '../health/provide-notification-host';
import { AlertsSource } from './alerts-source';
import { AlertsStore, kindLabel, MAX_ALERTS, MAX_TOASTS, MUTE_MS, muteKey, muteLabel, muteTarget, severityLabel, TOAST_MS } from './alerts.store';
import { AlertsView } from './alerts-view';
import { LiveAlerts } from './live-alerts';
import { SeverityBadge } from './severity-badge';
import { at } from '../../testing/must';

const rule = (name: string, patch: Partial<LiveAlert> = {}) => alert({ kind: 'rule', rule: name, instance: null, severity: 'warning', message: `Rule "${name}" matched`, ...patch });

describe('AlertsStore', () => {
  let backend: MockBackend;
  function setup() {
    backend = new MockBackend({ latency: false, dataset: tinyDataset });
    TestBed.configureTestingModule({ providers: provideTinyBackendBare(backend) });
    return TestBed.inject(AlertsStore);
  }

  afterEach(() => vi.useRealTimers());

  it('lists alerts newest first and toasts them, keeping a bounded number', () => {
    const store = setup();
    for (let i = 0; i < MAX_TOASTS + 2; i++) store.receive(alert({ message: `m${i}`, severity: 'info' }), 1000);
    expect(at(store.alerts(), 0).alert.message).toBe(`m${MAX_TOASTS + 1}`);
    expect(store.toasts().length).toBe(MAX_TOASTS);
    expect(store.toasts().at(-1)!.alert.message).toBe(`m${MAX_TOASTS + 1}`);
    for (let i = 0; i < MAX_ALERTS + 5; i++) store.receive(alert(), 1000);
    expect(store.alerts().length).toBe(MAX_ALERTS);
  });

  it('mutes a kind (or one rule) for 15 minutes: listed but not toasted, then toasted again', () => {
    const store = setup();
    const t0 = 10_000;
    const spike = alert();
    store.receive(spike, t0);
    store.receive(rule('payments'), t0);
    expect(store.toasts().length).toBe(2);
    store.mute(spike, t0);
    // Muting dismisses that kind's toasts only.
    expect(store.toasts().map((t) => t.alert.kind)).toEqual(['rule']);
    expect(store.isMuted(spike, t0 + 1)).toBe(true);
    const muted = store.receive(alert({ message: 'again' }), t0 + 60_000);
    expect(muted!.muted).toBe(true);
    expect(store.toasts().length).toBe(1);
    expect(at(store.alerts(), 0).muted).toBe(true);
    // Another rule is not muted by muting "payments".
    store.mute(rule('payments'), t0);
    expect(store.receive(rule('disk'), t0 + 1)!.muted).toBe(false);
    expect(store.receive(rule('payments'), t0 + 1)!.muted).toBe(true);
    expect(store.activeMutes(t0).map((m) => m.key)).toEqual(['errorSpike', 'rule:payments']);
    // After 15 minutes the mute has expired.
    const later = t0 + MUTE_MS + 1;
    expect(store.isMuted(spike, later)).toBe(false);
    expect(store.receive(alert(), later)!.muted).toBe(false);
    expect(store.activeMutes(later)).toEqual([]);
    expect(store.mutes()).toEqual({});
  });

  it('unmutes on request', () => {
    const store = setup();
    store.mute(alert(), 0);
    store.unmute('errorSpike');
    expect(store.isMuted(alert(), 1)).toBe(false);
  });

  it('drops an alert it already lists (same stable id)', () => {
    const store = setup();
    const a = alert({ id: 'errorSpike:WEB01:7' });
    expect(store.receive(a, 1000)).not.toBeNull();
    expect(store.receive({ ...a }, 2000)).toBeNull();
    expect(store.alerts().length).toBe(1);
    expect(store.toasts().length).toBe(1);
    expect(store.receive(alert({ id: 'errorSpike:WEB01:8' }), 3000)).not.toBeNull();
  });

  it('mutes and unmutes in the engine (kind or rule)', async () => {
    const store = setup();
    const mute = vi.spyOn(backend, 'muteAlerts');
    const unmute = vi.spyOn(backend, 'unmuteAlerts');
    const now = Date.now();
    await store.mute(alert(), now);
    expect(mute).toHaveBeenLastCalledWith({ kind: 'errorSpike' }, now + MUTE_MS);
    await store.mute(rule('payments'), now, 60_000);
    expect(mute).toHaveBeenLastCalledWith({ rule: 'payments' }, now + 60_000);
    await store.unmute('rule:payments');
    expect(unmute).toHaveBeenLastCalledWith({ rule: 'payments' });
    await store.unmute('errorSpike');
    expect(unmute).toHaveBeenLastCalledWith({ kind: 'errorSpike' });
    expect(muteTarget('rule:')).toEqual({ rule: '' });
    expect(muteTarget('silence')).toEqual({ kind: 'silence' });
  });

  it('keeps the local mute and logs when the engine refuses, never rejecting', async () => {
    const store = setup();
    vi.spyOn(backend, 'muteAlerts').mockRejectedValueOnce('the mute must end in the future');
    vi.spyOn(backend, 'unmuteAlerts').mockRejectedValueOnce('gone');
    const now = Date.now();
    await store.mute(alert(), now);
    expect(store.isMuted(alert(), now + 1)).toBe(true);
    await store.unmute('errorSpike');
    expect(store.isMuted(alert(), now + 1)).toBe(false);
    const warnings = testLogger().entries.filter((e) => e.level === 'warn').map((e) => e.message);
    expect(warnings).toEqual(['Muting live alerts in the engine failed', 'Unmuting live alerts in the engine failed']);
  });

  it('a muted kind is not raised by the (mock) engine', async () => {
    vi.useFakeTimers();
    const sim = new MockBackend({ latency: false, dataset: tinyDataset, liveRate: 4, liveAlertMs: 250, liveMs: 250 });
    const got: LiveAlert[] = [];
    await sim.onLiveAlert((a) => got.push(a));
    await sim.muteAlerts({ kind: 'errorSpike' }, Date.now() + 60_000);
    await sim.muteAlerts({ rule: 'payment errors' }, Date.now() + 60_000);
    await sim.setLive(true);
    await vi.advanceTimersByTimeAsync(250 * 6);
    await sim.setLive(false);
    expect(got.length).toBeGreaterThan(0);
    expect(got.every((a) => a.kind === 'silence')).toBe(true);
    await expect(sim.muteAlerts({ rule: ' ' }, Date.now() + 1)).rejects.toBe('name the alert rule to mute');
    await expect(sim.muteAlerts({ kind: 'crash' }, Date.now() - 1)).rejects.toBe('the mute must end in the future');
    await expect(sim.muteAlerts({ kind: 'crash' }, Date.now() + 2 * 86_400_000)).rejects.toBe('alerts can be muted for at most 24 hours');
    await sim.unmuteAlerts({ kind: 'errorSpike' });
  });

  it('dismisses warning and info toasts after a while; critical ones stay', async () => {
    vi.useFakeTimers();
    const store = setup();
    store.receive(alert({ severity: 'critical' }));
    store.receive(alert({ severity: 'warning' }));
    expect(store.toasts().length).toBe(2);
    await vi.advanceTimersByTimeAsync(TOAST_MS + 10);
    expect(store.toasts().map((t) => t.alert.severity)).toEqual(['critical']);
    store.dismissToast(at(store.toasts(), 0).id);
    expect(store.toasts()).toEqual([]);
  });

  it('filters by severity, counts, removes and clears', () => {
    const store = setup();
    store.receive(alert({ severity: 'critical' }));
    store.receive(alert({ severity: 'info' }));
    store.receive(alert({ severity: 'info' }));
    expect(store.counts()).toEqual({ critical: 1, warning: 0, info: 2 });
    store.setFilter('info');
    expect(store.visible().length).toBe(2);
    store.setFilter('nonsense' as never);
    expect(store.filter()).toBe('all');
    store.remove(at(store.alerts(), 0).id);
    expect(store.alerts().length).toBe(2);
    store.clear();
    expect(store.alerts()).toEqual([]);
    expect(store.toasts()).toEqual([]);
  });

  it('labels kinds, mutes and severities in words', () => {
    expect(kindLabel(alert())).toBe('Error spike');
    expect(kindLabel(rule('x'))).toBe('Rule: x');
    expect(kindLabel({ kind: 'rule', rule: null })).toBe('Rule: unnamed');
    expect(kindLabel({ kind: 'other' as never, rule: null })).toBe('other');
    expect(muteKey(rule('x'))).toBe('rule:x');
    expect(muteKey(alert({ kind: 'rule', rule: null }))).toBe('rule:');
    expect(muteLabel('rule:x')).toBe('Rule: x');
    expect(muteLabel('rule:')).toBe('Rule: unnamed');
    expect(muteLabel('silence')).toBe('Silence');
    expect(muteLabel('weird')).toBe('weird');
    expect(['critical', 'warning', 'info'].map((s) => severityLabel(s as never))).toEqual(['Critical', 'Warning', 'Info']);
  });
});

describe('LiveAlerts', () => {
  async function setup() {
    const backend = new MockBackend({ latency: false, dataset: tinyDataset });
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const ws = TestBed.inject(WorkspaceStore);
    await settle();
    return { backend, ws, alerts: TestBed.inject(LiveAlerts), store: TestBed.inject(AlertsStore) };
  }

  it('subscribes once to the alert event and records what arrives', async () => {
    const { backend, alerts, store } = await setup();
    let emit!: (a: LiveAlert) => void;
    const off = vi.fn();
    const sub = vi.spyOn(backend, 'onLiveAlert').mockImplementation(async (h) => ((emit = h), off));
    await alerts.start();
    await alerts.start();
    expect(sub).toHaveBeenCalledTimes(1);
    emit(alert());
    expect(store.alerts().length).toBe(1);
    expect(testLogger().entries.some((e) => e.message === 'Live alert' && e.props?.['Kind'] === 'errorSpike')).toBe(true);
    alerts.stop();
    expect(off).toHaveBeenCalled();
    sub.mockRejectedValueOnce('no events');
    await alerts.start();
    expect(testLogger().entries.some((e) => e.message === 'Live alerts unavailable')).toBe(true);
  });

  it('"Show" applies the evidence query and zooms to its time range', async () => {
    const { ws, alerts } = await setup();
    ws.setTextInput('old text');
    const a = alert({ evidenceQuery: { ...alert().evidenceQuery, text: 'timeout', from: 5_000, to: 9_000, window: { kind: 'live', lastMs: 1 } } });
    alerts.show(a);
    const q = ws.query();
    expect(q.levels).toEqual(['Error', 'Fatal']);
    expect(q.groups).toEqual(['WEB01 · 100']);
    expect(q.window).toBeUndefined();
    expect(ws.textInput()).toBe('timeout');
    expect(TestBed.inject(TimeCursor).selection()).toEqual({ from: 5_000, to: 9_000 });
    // The range comes from the alert when the evidence has none.
    alerts.show(alert({ from: 100, to: 100, evidenceQuery: { ...alert().evidenceQuery, from: null, to: null } }));
    expect(TestBed.inject(TimeCursor).selection()).toEqual({ from: 100, to: 101 });
  });
});

describe('alert components', () => {
  async function setup() {
    const backend = new MockBackend({ latency: false, dataset: tinyDataset });
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), provideNotificationSource(AlertsSource)] });
    await settle();
    return { backend, store: TestBed.inject(AlertsStore), actions: TestBed.inject(LiveAlerts) };
  }

  it('toasts show the severity in words, Show applies the evidence, Mute mutes the kind', async () => {
    const { store, actions } = await setup();
    const show = vi.spyOn(actions, 'show').mockImplementation(() => undefined);
    const fixture = TestBed.createComponent(NotificationHost);
    store.receive(alert());
    store.receive(rule('payments'));
    await settle(fixture);
    expect(all(fixture, '[data-testid=alert-toast]').length).toBe(2);
    expect(text(fixture)).toContain('Critical');
    expect(text(fixture)).toContain('Error spike');
    expect(text(fixture)).toContain('WEB01 · 100');
    buttonByText(fixture, 'Show').click();
    await settle(fixture);
    expect(show).toHaveBeenCalledWith(expect.objectContaining({ kind: 'errorSpike' }));
    expect(all(fixture, '[data-testid=alert-toast]').length).toBe(1);
    buttonByText(fixture, 'Mute this kind for 15 min').click();
    await settle(fixture);
    expect(store.isMuted(rule('payments'))).toBe(true);
    store.receive(alert({ severity: 'info' }));
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '.x').click();
    await settle(fixture);
    expect(all(fixture, '[data-testid=alert-toast]').length).toBe(0);
  });

  it('the alerts panel lists, filters, mutes, shows evidence, clears and opens the rules editor', async () => {
    const { store, actions } = await setup();
    const show = vi.spyOn(actions, 'show').mockImplementation(() => undefined);
    const open = vi.spyOn(TestBed.inject(DialogService), 'open');
    const fixture = TestBed.createComponent(AlertsView);
    await settle(fixture);
    expect(text(fixture)).toContain('No alerts yet');
    expect(text(fixture)).toContain('Alerts are raised while live tail is on');
    store.receive(alert());
    store.receive(rule('payments', { severity: 'info' }));
    await settle(fixture);
    expect(all(fixture, '[data-testid=alert-row]').length).toBe(2);
    expect(text(fixture)).toContain('All 2');
    buttonByText(fixture, /Critical 1/).click();
    await settle(fixture);
    expect(all(fixture, '[data-testid=alert-row]').length).toBe(1);
    buttonByText(fixture, /Warning 0/).click();
    await settle(fixture);
    expect(text(fixture)).toContain('No warning alerts');
    buttonByText(fixture, /All 2/).click();
    await settle(fixture);
    all<HTMLButtonElement>(fixture, 'button').find((b) => b.getAttribute('aria-label') === 'Show evidence: Error spike')!.click();
    expect(show).toHaveBeenCalled();
    all<HTMLButtonElement>(fixture, 'button').find((b) => b.getAttribute('aria-label') === 'Mute 15 min: Error spike')!.click();
    await settle(fixture);
    expect(text(fixture)).toContain('Muted:');
    expect(text(fixture)).toContain('Error spike until');
    buttonByText(fixture, 'Unmute').click();
    await settle(fixture);
    expect(text(fixture)).not.toContain('Muted:');
    store.receive(alert({ message: 'muted one' }));
    store.mute(alert());
    store.receive(alert({ message: 'muted two' }));
    await settle(fixture);
    expect(text(fixture)).toContain('(muted)');
    all<HTMLButtonElement>(fixture, 'button').find((b) => b.getAttribute('aria-label') === 'Remove alert Rule: payments')!.click();
    await settle(fixture);
    expect(store.alerts().some((e) => e.alert.kind === 'rule')).toBe(false);
    buttonByText(fixture, 'Alert rules…').click();
    expect(open).toHaveBeenCalled();
    buttonByText(fixture, 'Clear').click();
    await settle(fixture);
    expect(text(fixture)).toContain('No alerts yet');
    TestBed.inject(LiveTail).enabled.set(true);
    await settle(fixture);
    expect(text(fixture)).not.toContain('Alerts are raised while live tail is on');
    expect(el(fixture, 'td, .empty-state')).toBeTruthy();
  });

  it('the severity badge has a distinct shape and a label per severity', async () => {
    TestBed.configureTestingModule({});
    const fixture = TestBed.createComponent(SeverityBadge);
    fixture.componentRef.setInput('severity', 'warning');
    await settle(fixture);
    expect(text(fixture)).toContain('Warning');
    const warn = el(fixture, 'path').getAttribute('d');
    fixture.componentRef.setInput('severity', 'critical');
    fixture.componentRef.setInput('showLabel', false);
    await settle(fixture);
    expect(el(fixture, '.sr').textContent).toBe('Critical');
    expect(el(fixture, 'path').getAttribute('d')).not.toBe(warn);
    fixture.componentRef.setInput('severity', 'bogus');
    await settle(fixture);
    expect(el(fixture, 'path').getAttribute('d')).toContain('a9 9');
  });
});
