import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { InstalledPluginDto, InstalledPluginsDto, PackageReviewDto, PermissionDto } from '../core/api.types';
import { Platform } from '../core/platform';
import { LifecycleBackend, PluginPackagesBackend } from '../core/tools-backend';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { errorMessage } from '../tools/tool-errors';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { DialogService } from '../ui/dialog.service';

/** The file extension of a plugin package. */
export const PACKAGE_EXTENSION = 'debriefplugin';

/** Whether a path names a plugin package (by its extension, any case). */
export function isPackagePath(path: string): boolean {
  return path.toLowerCase().endsWith(`.${PACKAGE_EXTENSION}`);
}

/** A package being reviewed before installing: what the shell read from it and what the user approves. */
export interface PackageReview {
  path: string;
  pkg: PackageReviewDto;
  /** The requested permissions the user grants (all of them by default). */
  approved: PermissionDto[];
}

interface PackagesState {
  installed: InstalledPluginsDto | null;
  loading: boolean;
  /** What is being done right now (shown on the busy control), or null. */
  busy: string | null;
  error: string | null;
  /** The failed request that Retry repeats, or null when retrying makes no sense. */
  retry: 'load' | null;
  review: PackageReview | null;
  /** Something was installed or changed that applies only after a restart. */
  restartNeeded: boolean;
}

const INITIAL: PackagesState = { installed: null, loading: false, busy: null, error: null, retry: null, review: null, restartNeeded: false };

/**
 * Third-party plugins installed from `.debriefplugin` packages: the review before installing (signature, permissions),
 * then enable/disable, permissions, quarantine reset and removal. Failures never throw out of the store; they land in
 * `error`.
 */
export const PluginPackagesStore = signalStore(
  { providedIn: 'root' },
  withState<PackagesState>(INITIAL),
  withComputed((store) => ({
    plugins: computed<InstalledPluginDto[]>(() => store.installed()?.plugins ?? []),
    damaged: computed(() => store.installed()?.damaged ?? []),
    safeMode: computed(() => !!store.installed()?.safeMode),
    developerMode: computed(() => !!store.installed()?.developerMode),
    /** Changes wait for a restart: one made in this run, or a plugin the shell reports as pending. */
    showRestart: computed(() => store.restartNeeded() || (store.installed()?.plugins ?? []).some((p) => p.state === 'pendingRestart')),
    /** The reviewed package may be installed (the shell did not refuse it). */
    canInstall: computed(() => !!store.review() && !store.review()?.pkg.refusal && !store.busy()),
  })),
  withMethods(
    (
      store,
      backend = inject(PluginPackagesBackend),
      lifecycle = inject(LifecycleBackend),
      life = inject(LifecycleService),
      platform = inject(Platform),
      dialogs = inject(DialogService),
    ) => {
      /** Runs an action with the busy label; a failure lands in `error`. Resolves to whether it succeeded. */
      const run = async (label: string, op: () => Promise<void>): Promise<boolean> => {
        patchState(store, { busy: label, error: null, retry: null });
        try {
          await op();
          return true;
        } catch (e) {
          patchState(store, { error: errorMessage(e) });
          return false;
        } finally {
          patchState(store, { busy: null });
        }
      };
      /** Replaces one plugin in the list with what the shell returned. */
      const replace = (plugin: InstalledPluginDto) =>
        patchState(store, (s) => (s.installed ? { installed: { ...s.installed, plugins: s.installed.plugins.map((p) => (p.id === plugin.id ? plugin : p)) } } : {}));
      const load = async (): Promise<void> => {
        patchState(store, { loading: true });
        try {
          patchState(store, { installed: await backend.listPackages() });
          if (store.retry() === 'load') patchState(store, { error: null, retry: null });
        } catch (e) {
          patchState(store, { error: errorMessage(e), retry: 'load' });
        } finally {
          patchState(store, { loading: false });
        }
      };
      const review = async (path: string): Promise<void> => {
        await run($localize`:@@pluginsPanel.packages.busy.reading:Checking the package…`, async () => {
          const pkg = await backend.readPackage(path);
          patchState(store, { review: { path, pkg, approved: pkg.refusal ? [] : pkg.permissions.map((p) => p.permission) } });
        });
      };
      return {
        /** Reads what is installed. Never rejects. */
        load,
        /** Asks for a package in the native dialog, then reviews it. */
        pickAndReview: async (): Promise<void> => {
          let path: string | null;
          try {
            path = await platform.pickOpenPath($localize`:@@pluginsPanel.packages.pickTitle:Install a plugin`, [
              { name: $localize`:@@pluginsPanel.packages.pickFilter:Debrief plugin package`, extensions: [PACKAGE_EXTENSION] },
            ]);
          } catch (e) {
            patchState(store, { error: errorMessage(e), retry: null });
            return;
          }
          if (path) await review(path);
        },
        /** Reads and checks a package (a dropped or chosen path) for the review dialog. */
        reviewPackage: review,
        /** Grants or withholds one requested permission in the review. */
        toggleApproved: (permission: PermissionDto) =>
          patchState(store, (s) => {
            if (!s.review) return {};
            const approved = s.review.approved.includes(permission) ? s.review.approved.filter((p) => p !== permission) : [...s.review.approved, permission];
            return { review: { ...s.review, approved } };
          }),
        /** Installs the reviewed package with the approved permissions; the review closes when it succeeded. */
        install: async (): Promise<void> => {
          const current = store.review();
          if (!current || current.pkg.refusal) return;
          const done = await run($localize`:@@pluginsPanel.packages.busy.installing:Installing…`, async () => {
            const approved = current.pkg.permissions.map((p) => p.permission).filter((p) => current.approved.includes(p));
            await backend.installPackage(current.path, current.pkg.digest, approved);
          });
          if (!done) return;
          patchState(store, { review: null, restartNeeded: true });
          await load();
        },
        cancelReview: () => patchState(store, { review: null, error: null, retry: null }),
        /** Enables or disables an installed plugin. */
        setEnabled: async (id: string, enabled: boolean): Promise<void> => {
          await run(enabled ? $localize`:@@pluginsPanel.packages.busy.enabling:Enabling…` : $localize`:@@pluginsPanel.packages.busy.disabling:Disabling…`, async () =>
            replace(await backend.setPackageEnabled(id, enabled)),
          );
        },
        /** Grants exactly `granted`; a running plugin restarts with them at once. Resolves to whether it succeeded. */
        setPermissions: (id: string, granted: PermissionDto[]): Promise<boolean> =>
          run($localize`:@@pluginsPanel.packages.busy.permissions:Changing permissions…`, async () => replace(await backend.setPackagePermissions(id, granted))),
        /** Asks, then uninstalls. A plugin that was loaded is gone after a restart. */
        remove: async (id: string): Promise<void> => {
          const name = store.plugins().find((p) => p.id === id)?.name ?? id;
          const ref = dialogs.open<boolean>(ConfirmDialog, {
            title: $localize`:@@pluginsPanel.packages.remove.title:Remove plugin`,
            inputs: {
              message: $localize`:@@pluginsPanel.packages.remove.message:Remove ${name}:name:? Its files are deleted; install the package again to get it back.`,
              detail: id,
              confirmLabel: $localize`:@@pluginsPanel.packages.remove.confirm:Remove`,
              danger: true,
            },
          });
          if ((await ref.closed) !== true) return;
          const loaded = store.plugins().some((p) => p.id === id && (p.state === 'running' || p.state === 'ready'));
          const done = await run($localize`:@@pluginsPanel.packages.busy.removing:Removing…`, async () => {
            await backend.removePackage(id);
          });
          if (!done) return;
          if (loaded) patchState(store, { restartNeeded: true });
          await load();
        },
        /** Lifts a quarantine (the plugin is called again), then reads the list again. */
        reset: async (id: string): Promise<void> => {
          if (await run($localize`:@@pluginsPanel.packages.busy.resetting:Resetting…`, () => lifecycle.pluginReset(id))) await load();
        },
        /** Relaunches so the changes apply; the session is reopened. */
        restart: () => life.restartKeepingSession(),
        /** Repeats the failed request (only reading the list can be retried). */
        retryFailed: async (): Promise<void> => {
          if (store.retry() === 'load') await load();
        },
        dismissError: () => patchState(store, { error: null, retry: null }),
      };
    },
  ),
);
