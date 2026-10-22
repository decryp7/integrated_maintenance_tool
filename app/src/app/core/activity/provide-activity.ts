import { DestroyRef, EnvironmentProviders, inject, makeEnvironmentProviders, provideAppInitializer } from '@angular/core';
import { isTauri } from '../platform';
import { ActivityLog } from './activity-log';
import { invoke } from './ipc';
import { ActivityObservers } from './observers';
import { Detail } from './privacy';

/** The native menu's click event (macOS), see `shell/menu/native-menu.types.ts`. */
const MENU_EVENT = 'debrief://menu';

/** Reads the activity detail setting (Settings ▸ Diagnostics ▸ Activity log detail) from the engine. */
export async function loadDetail(call: typeof invoke = invoke): Promise<Detail> {
  const s = await call<{ activity?: { detail?: unknown } }>('diagnostics_settings');
  return s?.activity?.detail === 'detailed' ? 'detailed' : 'normal';
}

/** Starts the activity log of this window. Never throws: the activity log must not stop the app. */
export function startActivity(): void {
  const log = inject(ActivityLog);
  const destroy = inject(DestroyRef);
  const runtime = log.runtime;
  try {
    const stop = runtime.triggers.install(document);
    destroy.onDestroy(stop);
    inject(ActivityObservers).start();
  } catch {
    /* without input tracking, triggers are inferred as "program" */
  }
  if (!isTauri()) return;
  const refresh = () =>
    loadDetail().then(
      (d) => log.setDetail(d),
      () => undefined,
    );
  void refresh();
  // A settings change may switch the detail level.
  destroy.onDestroy(runtime.subscribe((b) => (b.operation === 'settings.change' || b.operation === 'settings.reset' ? void refresh() : undefined)));
  void import('../../windowing/window-events')
    .then(({ listenHere }) => listenHere(MENU_EVENT, (id) => runtime.triggers.noteNativeMenu(typeof id === 'string' ? id : String((id as { commandId?: unknown })?.commandId ?? ''))))
    .then((off) => destroy.onDestroy(off))
    .catch(() => undefined);
}

/**
 * The activity log (`core/activity`): binds it to the logger, tracks user input for trigger attribution, watches
 * the stores and the dock, and reads the detail setting. Add once in `app.config.ts`.
 */
export function provideActivity(): EnvironmentProviders {
  return makeEnvironmentProviders([provideAppInitializer(() => startActivity())]);
}
