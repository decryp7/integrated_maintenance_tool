import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { EventLogBackend } from '../../core/tools-backend';
import { EventLogChannel, EventLogLevel, EventLogStatus, EventLogValidation, isEventLogRun, TOOL_IDS } from '../../core/tools.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { LifecycleService } from '../../lifecycle/lifecycle.service';
import { LiveTail } from '../../workspace/live-tail';
import { ToolCatalog } from '../tool-catalog.store';
import { errorMessage } from '../tool-errors';
import { DEFAULT_EVENT_LOG_DRAFT, EventLogDraft, eventLogProblems, filterChannels, toEventLogQuery, toEventLogStart, toggled } from './eventlog-model';

interface EventLogState {
  /** The channels the engine offers (empty until loaded). */
  channels: EventLogChannel[];
  search: string;
  draft: EventLogDraft;
  /** The last validation of the current draft; cleared when the draft changes. */
  validation: EventLogValidation | null;
  status: EventLogStatus | null;
  /** Folder of the current or last run. */
  dir: string | null;
  /** Channels the current run continues from their saved position. */
  resumed: string[];
  busy: string | null;
  error: string | null;
  notice: string | null;
}

/** State and actions of the Event Log follower panel. The engine follows and validates; the UI configures and shows. */
export const EventLogStore = signalStore(
  { providedIn: 'root' },
  withState<EventLogState>({
    channels: [],
    search: '',
    draft: DEFAULT_EVENT_LOG_DRAFT,
    validation: null,
    status: null,
    dir: null,
    resumed: [],
    busy: null,
    error: null,
    notice: null,
  }),
  withComputed((store, catalog = inject(ToolCatalog), life = inject(LifecycleService)) => {
    const run = computed(() => {
      const s = store.status();
      return isEventLogRun(s) ? s : null;
    });
    return {
      /** The tools plugin serves the follower here (Windows, plugin active). */
      canRun: computed(() => catalog.usable(TOOL_IDS.eventLog)),
      /** Why the follower cannot run on this platform, from the engine; null when it can. */
      unsupported: computed(() => {
        const a = catalog.entry(TOOL_IDS.eventLog)?.availability;
        return a?.state === 'unsupported' ? a.reason : null;
      }),
      run,
      running: computed(() => run()?.running === true),
      paused: computed(() => run()?.paused === true),
      problems: computed(() => eventLogProblems(store.draft())),
      visibleChannels: computed(() => filterChannels(store.channels(), store.search())),
      /** Chosen channels that need administrator rights IMT does not have. */
      needsAdmin: computed(() =>
        life.elevated() ? [] : store.channels().filter((c) => c.needsAdmin && store.draft().channels.includes(c.name)).map((c) => c.name),
      ),
    };
  }),
  withMethods((store, backend = inject(EventLogBackend), live = inject(LiveTail), workspace = inject(WorkspaceStore), catalog = inject(ToolCatalog)) => {
    const run = async (label: string, op: () => Promise<void>): Promise<void> => {
      patchState(store, { busy: label, error: null, notice: null });
      try {
        await op();
      } catch (e) {
        patchState(store, { error: errorMessage(e) });
      } finally {
        patchState(store, { busy: null });
      }
    };
    const showStatus = (status: EventLogStatus) => patchState(store, { status, dir: status.dir ?? store.dir() });
    const patchDraft = (patch: Partial<EventLogDraft>) => patchState(store, (s) => ({ draft: { ...s.draft, ...patch }, validation: null }));

    return {
      patchDraft,
      setSearch: (search: string) => patchState(store, { search }),
      toggleChannel: (name: string, on: boolean) => patchDraft({ channels: toggled(store.draft().channels, name, on) }),
      toggleLevel: (level: EventLogLevel, on: boolean) => patchDraft({ levels: toggled(store.draft().levels, level, on) }),
      clearChannels: () => patchDraft({ channels: [] }),
      dismissError: () => patchState(store, { error: null }),

      /** Reads the channels and whether a run is in progress (the follower outlives the panel). */
      load: () =>
        run($localize`:@@tools.eventlog.busy.loading:Loading channels…`, async () => {
          // Not on this platform or with the plugin unavailable: the panel explains why instead of failing.
          if (!catalog.loaded()) await catalog.load();
          if (!store.canRun()) return;
          const [channels, status] = await Promise.all([backend.eventLogChannels(), backend.eventLogStatus()]);
          patchState(store, { channels });
          showStatus(status);
        }),

      /** Reads the status without the busy indicator (polling while a run is going). */
      poll: async (): Promise<void> => {
        try {
          showStatus(await backend.eventLogStatus());
        } catch (e) {
          patchState(store, { error: errorMessage(e) });
        }
      },

      /** Asks the engine to compose the queries and check each chosen channel. */
      validate: () =>
        run($localize`:@@tools.eventlog.busy.validating:Validating…`, async () => {
          patchState(store, { validation: await backend.eventLogValidate(toEventLogQuery(store.draft())) });
        }),

      /** Starts following, opens the output folder and follows it live. */
      start: () =>
        run($localize`:@@tools.eventlog.busy.starting:Starting…`, async () => {
          const started = await backend.eventLogStart(toEventLogStart(store.draft()));
          patchState(store, { dir: started.dir, resumed: started.resumed });
          showStatus(await backend.eventLogStatus());
          await workspace.openPaths([started.dir]);
          await live.start();
          const names = started.resumed.join(', ');
          const notice = started.resumed.length
            ? $localize`:@@tools.eventlog.notice.startedResumed:Following started. The events appear in the workspace as they are written. Continuing from the last position of ${names}:channels:.`
            : $localize`:@@tools.eventlog.notice.started:Following started. The events appear in the workspace as they are written.`;
          patchState(store, { notice });
        }),

      pause: () => run($localize`:@@tools.eventlog.busy.pausing:Pausing…`, async () => showStatus(await backend.eventLogPause())),
      resume: () => run($localize`:@@tools.eventlog.busy.resuming:Resuming…`, async () => showStatus(await backend.eventLogResume())),

      stop: () =>
        run($localize`:@@tools.eventlog.busy.stopping:Stopping…`, async () => {
          const dir = await backend.eventLogStop();
          // The last counts stay visible, marked as stopped.
          patchState(store, (s) => ({
            dir: dir ?? s.dir,
            status: isEventLogRun(s.status) ? { ...s.status, running: false, paused: false } : s.status,
            resumed: [],
            notice: $localize`:@@tools.eventlog.notice.stopped:Following stopped. The positions are saved for the next run.`,
          }));
        }),
    };
  }),
);
