import { inject, Injectable } from '@angular/core';
import { LiveOptions, MAX_DETECT_EVERY_MS } from '../core/api.types';
import { UiStateContribution } from '../lifecycle/ui-state';
import { DEFAULT_LIVE_OPTIONS, LiveTail } from '../workspace/live-tail';
import { LiveFollow } from './live-follow.store';

const posInt = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) && v > 0 ? Math.round(v) : null);

/**
 * A detector interval (ms) within the range the engine accepts (0 .. MAX_DETECT_EVERY_MS, whole ms), from a number or
 * the text of an input; null when it is not a number. The engine clamps again.
 */
export function parseDetectEvery(value: unknown): number | null {
  const n = typeof value === 'string' ? (/^\s*\d+(\.\d+)?\s*$/.test(value) ? Number(value) : NaN) : typeof value === 'number' ? value : NaN;
  if (!Number.isFinite(n) || n < 0) return null;
  return Math.min(MAX_DETECT_EVERY_MS, Math.round(n));
}

/** Validates saved live settings (untrusted project/session state) on top of the defaults. */
export function sanitizeLiveOptions(value: unknown): LiveOptions {
  const v = value && typeof value === 'object' ? (value as Record<string, unknown>) : {};
  const retention = v['retention'] && typeof v['retention'] === 'object' ? (v['retention'] as Record<string, unknown>) : {};
  const carry = typeof v['carryFlushMs'] === 'number' && Number.isFinite(v['carryFlushMs']) && v['carryFlushMs'] >= 0 ? Math.round(v['carryFlushMs']) : DEFAULT_LIVE_OPTIONS.carryFlushMs;
  const detect = parseDetectEvery(v['detectEveryMs']) ?? DEFAULT_LIVE_OPTIONS.detectEveryMs;
  return {
    watch: v['watch'] === 'events' || v['watch'] === 'polling' ? v['watch'] : 'auto',
    pollIntervalMs: posInt(v['pollIntervalMs']) ?? DEFAULT_LIVE_OPTIONS.pollIntervalMs,
    deletedFiles: v['deletedFiles'] === 'drop' ? 'drop' : 'keep',
    retention: { maxRecords: posInt(retention['maxRecords']), maxAgeMs: posInt(retention['maxAgeMs']) },
    carryFlushMs: carry,
    detectEveryMs: detect,
  };
}

/** Live settings saved with the project and the autosaved session: follow/window and the `set_live` options. */
@Injectable({ providedIn: 'root' })
export class LiveUiState {
  private readonly live = inject(LiveTail);
  private readonly follow = inject(LiveFollow);

  readonly contributions: UiStateContribution[] = [
    {
      key: 'live',
      capture: () => ({
        follow: { follow: this.follow.follow(), preset: this.follow.preset(), customMinutes: this.follow.customMinutes() },
        options: this.live.options(),
      }),
      restore: (v) => {
        const saved = (v && typeof v === 'object' ? v : {}) as { follow?: unknown; options?: unknown };
        this.follow.restore(saved.follow);
        void this.live.setOptions(sanitizeLiveOptions(saved.options));
      },
      reset: () => {
        this.follow.reset();
        void this.live.setOptions(DEFAULT_LIVE_OPTIONS);
      },
    },
  ];
}
