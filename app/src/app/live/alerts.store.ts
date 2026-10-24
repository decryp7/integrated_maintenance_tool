import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { AlertTarget, AnomalyKind, LiveAlert, Severity, SEVERITIES } from '../core/api.types';
import { DebriefBackend } from '../core/backend';
import { describeError, Logger } from '../core/logger';

/** Alerts kept in the panel (oldest dropped first). */
export const MAX_ALERTS = 500;
/** Toasts on screen at once (older ones are dismissed). */
export const MAX_TOASTS = 3;
/** "Mute this kind" lasts this long. */
export const MUTE_MS = 15 * 60_000;

export interface AlertEntry {
  id: number;
  alert: LiveAlert;
  /** Wall-clock time the UI received it. */
  receivedAt: number;
  /** It arrived while its kind was muted (listed, but no toast). */
  muted: boolean;
}

export type SeverityFilter = Severity | 'all';

interface AlertsState {
  /** Newest first. */
  alerts: AlertEntry[];
  toasts: AlertEntry[];
  filter: SeverityFilter;
  /** Mute key → wall-clock time (ms) the mute ends. */
  mutes: Record<string, number>;
}

/** What "Mute this kind" mutes: an anomaly kind, or one alert rule. */
export function muteKey(alert: LiveAlert): string {
  return alert.kind === 'rule' ? `rule:${alert.rule ?? ''}` : alert.kind;
}

/** The engine's mute target of a mute key (`rule:<name>` or an anomaly kind). */
export function muteTarget(key: string): AlertTarget {
  return key.startsWith('rule:') ? { rule: key.slice(5) } : { kind: key as AnomalyKind };
}

const KIND_LABELS: Record<string, string> = {
  errorSpike: $localize`:@@live.alerts.kind.errorSpike:Error spike`,
  silence: $localize`:@@live.alerts.kind.silence:Silence`,
  crash: $localize`:@@live.alerts.kind.crash:Crash`,
  slowOutlier: $localize`:@@live.alerts.kind.slowOutlier:Slow outlier`,
  rateChange: $localize`:@@live.alerts.kind.rateChange:Rate change`,
  memoryGrowth: $localize`:@@live.alerts.kind.memoryGrowth:Memory growth`,
};

const ruleKind = (name: string): string => $localize`:@@live.alerts.ruleKind:Rule: ${name}:name:`;

/** Display name of an alert's kind ("Rule: payment errors" for rules). */
export function kindLabel(alert: Pick<LiveAlert, 'kind' | 'rule'>): string {
  if (alert.kind === 'rule') return ruleKind(alert.rule ?? $localize`:@@live.alerts.unnamedRule:unnamed`);
  return KIND_LABELS[alert.kind] ?? alert.kind;
}

/** Display name of a mute key. */
export function muteLabel(key: string): string {
  return key.startsWith('rule:') ? ruleKind(key.slice(5) || $localize`:@@live.alerts.unnamedRule:unnamed`) : (KIND_LABELS[key] ?? key);
}

export function severityLabel(s: Severity): string {
  return s === 'critical' ? $localize`:@@live.alerts.severity.critical:Critical` : s === 'warning' ? $localize`:@@live.alerts.severity.warning:Warning` : $localize`:@@live.alerts.severity.info:Info`;
}

/**
 * Alerts raised by live tail: the panel list, the toasts, the severity filter and per-kind mutes. Alerts are
 * de-duplicated by their stable engine id. Mutes are applied in the engine (`mute_alerts`: muted alerts are not
 * raised at all) and mirrored here for the list of active mutes and for alerts already on their way.
 */
export const AlertsStore = signalStore(
  { providedIn: 'root' },
  withState<AlertsState>({ alerts: [], toasts: [], filter: 'all', mutes: {} }),
  withComputed((store) => ({
    visible: computed(() => {
      const f = store.filter();
      return f === 'all' ? store.alerts() : store.alerts().filter((e) => e.alert.severity === f);
    }),
    counts: computed(() => {
      const counts: Record<Severity, number> = { critical: 0, warning: 0, info: 0 };
      for (const e of store.alerts()) counts[e.alert.severity]++;
      return counts;
    }),
  })),
  withMethods((store, backend = inject(DebriefBackend), logger = inject(Logger)) => {
    let nextId = 1;

    const dismissToast = (id: number) => {
      patchState(store, (s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
    };

    /** Active mutes only (expired ones are dropped). */
    const liveMutes = (now: number): Record<string, number> => Object.fromEntries(Object.entries(store.mutes()).filter(([, until]) => until > now));

    return {
      /** Records an alert; shows a toast unless its kind is muted. Null for an alert already listed (same id). */
      receive(alert: LiveAlert, now = Date.now()): AlertEntry | null {
        if (store.alerts().some((e) => e.alert.id === alert.id)) return null;
        const mutes = liveMutes(now);
        const entry: AlertEntry = { id: nextId++, alert, receivedAt: now, muted: (mutes[muteKey(alert)] ?? 0) > now };
        const dropped = entry.muted ? [] : store.toasts().slice(0, Math.max(0, store.toasts().length - (MAX_TOASTS - 1)));
        for (const t of dropped) dismissToast(t.id);
        patchState(store, (s) => ({
          alerts: [entry, ...s.alerts].slice(0, MAX_ALERTS),
          toasts: entry.muted ? s.toasts : [...s.toasts, entry],
          mutes,
        }));
        return entry;
      },
      dismissToast,
      /**
       * Mutes this alert's kind (or rule) for `ms` in the engine, dismisses the toasts of that kind, and lists the
       * mute. Never rejects: when the engine refuses, the mute still applies to toasts here and the reason is logged.
       */
      async mute(alert: LiveAlert, now = Date.now(), ms = MUTE_MS): Promise<void> {
        const key = muteKey(alert);
        for (const t of store.toasts()) if (muteKey(t.alert) === key) dismissToast(t.id);
        patchState(store, { mutes: { ...liveMutes(now), [key]: now + ms } });
        try {
          await backend.muteAlerts(muteTarget(key), now + ms);
        } catch (e) {
          logger.warn('ui.live', 'Muting live alerts in the engine failed', { Error: describeError(e) });
        }
      },
      /** Lifts a mute here and in the engine. Never rejects. */
      async unmute(key: string): Promise<void> {
        const { [key]: _gone, ...rest } = store.mutes();
        patchState(store, { mutes: rest });
        try {
          await backend.unmuteAlerts(muteTarget(key));
        } catch (e) {
          logger.warn('ui.live', 'Unmuting live alerts in the engine failed', { Error: describeError(e) });
        }
      },
      isMuted(alert: LiveAlert, now = Date.now()): boolean {
        return (store.mutes()[muteKey(alert)] ?? 0) > now;
      },
      /** Active mutes with their end time, soonest first. */
      activeMutes(now = Date.now()): { key: string; until: number }[] {
        return Object.entries(liveMutes(now))
          .map(([key, until]) => ({ key, until }))
          .sort((a, b) => a.until - b.until);
      },
      setFilter(filter: SeverityFilter): void {
        patchState(store, { filter: filter === 'all' || SEVERITIES.includes(filter) ? filter : 'all' });
      },
      remove(id: number): void {
        dismissToast(id);
        patchState(store, (s) => ({ alerts: s.alerts.filter((e) => e.id !== id) }));
      },
      clear(): void {
        for (const t of store.toasts()) dismissToast(t.id);
        patchState(store, { alerts: [] });
      },
    };
  }),
);

export type AlertsStoreType = InstanceType<typeof AlertsStore>;
