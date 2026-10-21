import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { ParserEntryDto, ParserLibraryDto } from '../../core/api.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { errorMessage } from '../../tools/tool-errors';
import { OverwriteGuard } from '../../ui/overwrite-guard';
import { ParserDialogs } from './parser-dialogs';
import { ParserFiles } from './parser-files';
import { ParserSaveOptions, ParsersBackend } from './parsers-backend';

/** What "New blank" starts from: the smallest useful parser (the engine's built-in `generic` documents every key). */
export const BLANK_PARSER = String.raw`schema = 1
id = "my-format"
name = "My format"
version = "1.0.0"
description = ""
category = "Custom"

[match]
files = ["*.log"]

[format]
kind = "text"
regexes = ['^(?P<timestamp>\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}(?:[.,]\d+)?)\s+(?P<level>\w+)\s+(?P<message>.*)$']
`;

interface ParsersState {
  library: ParserLibraryDto | null;
  loading: boolean;
  /** Listing failed (shown with Retry). */
  loadError: string | null;
  /** The parser an action runs on now ('*' for the library: import, re-read). */
  busy: string | null;
  /** What the last action did. */
  notice: string | null;
  /** The last action failed. */
  actionError: string | null;
}

const INITIAL: ParsersState = { library: null, loading: false, loadError: null, busy: null, notice: null, actionError: null };

/** Parsers of one category, for the grouped list. */
export interface ParserGroup {
  category: string;
  parsers: ParserEntryDto[];
}

/**
 * State of Settings ▸ Parsers (ADR 0026): the library as the engine lists it, and the outcome of the actions on it
 * (enable, remove, duplicate, edit, import, export, re-read, "Parse with…"). The engine decides everything; the
 * store asks the user through `ParserDialogs` and shows what happened.
 */
export const ParsersStore = signalStore(
  { providedIn: 'root' },
  withState<ParsersState>(INITIAL),
  withComputed((store) => {
    const parsers = computed(() => store.library()?.parsers ?? []);
    return {
      parsers,
      broken: computed(() => store.library()?.broken ?? []),
      stale: computed(() => store.library()?.stale ?? 0),
      groups: computed<ParserGroup[]>(() => {
        const byCategory = new Map<string, ParserEntryDto[]>();
        for (const p of parsers()) byCategory.set(p.category, [...(byCategory.get(p.category) ?? []), p]);
        return [...byCategory].map(([category, list]) => ({ category, parsers: list }));
      }),
      /** For "Parse with…": enabled parsers first, each part by name. */
      choices: computed(() => [...parsers()].sort((a, b) => Number(b.enabled) - Number(a.enabled) || a.name.localeCompare(b.name))),
    };
  }),
  withMethods((store, backend = inject(ParsersBackend), dialogs = inject(ParserDialogs), files = inject(ParserFiles), guard = inject(OverwriteGuard), workspace = inject(WorkspaceStore)) => {
    const entry = (id: string) => store.parsers().find((p) => p.id === id) ?? null;

    const load = async (): Promise<void> => {
      patchState(store, { loading: true, loadError: null });
      try {
        patchState(store, { library: await backend.list() });
      } catch (e) {
        patchState(store, { loadError: errorMessage(e) });
      } finally {
        patchState(store, { loading: false });
      }
    };

    /** Runs one action: busy while it runs, its notice or error after. */
    const act = async (busy: string, work: () => Promise<string | null>): Promise<void> => {
      patchState(store, { busy, notice: null, actionError: null });
      try {
        const notice = await work();
        if (notice) patchState(store, { notice });
      } catch (e) {
        patchState(store, { actionError: errorMessage(e) });
      } finally {
        patchState(store, { busy: null });
      }
    };

    const edited = (library: ParserLibraryDto | undefined, id: string): string | null => {
      if (!library) return null;
      patchState(store, { library });
      return $localize`:@@settings.parsers.notice.saved:Saved the parser ${id}:id:.`;
    };

    return {
      load,

      setEnabled: (id: string, enabled: boolean) =>
        act(id, async () => {
          patchState(store, { library: await backend.setEnabled(id, enabled) });
          return null;
        }),

      /** Removes a user parser or an override, after the user confirmed. */
      remove: (id: string) =>
        act(id, async () => {
          const e = entry(id);
          if (!e || e.origin === 'builtin' || !(await dialogs.confirmRemove(e))) return null;
          patchState(store, { library: await backend.remove(id) });
          return e.origin === 'override'
            ? $localize`:@@settings.parsers.notice.overrideRemoved:Removed your override of ${e.name}:name:; the built-in parser is used again.`
            : $localize`:@@settings.parsers.notice.removed:Removed the parser ${e.name}:name:.`;
        }),

      duplicate: (id: string) =>
        act(id, async () => {
          const copy = await backend.duplicate(id);
          patchState(store, { library: await backend.list() });
          return $localize`:@@settings.parsers.notice.duplicated:Saved a copy as ${copy.id}:id:.`;
        }),

      /** Opens the parser in the TOML editor (a built-in saves as an override). */
      edit: (id: string) =>
        act(id, async () => {
          const doc = await backend.get(id);
          return edited(await dialogs.edit(doc, { readSources: entry(id)?.sources.map((s) => s.id) ?? [] }), id);
        }),

      /** "New blank": the editor with a minimal parser. */
      newBlank: () =>
        act('*', async () => {
          const library = await dialogs.edit({ id: '', origin: 'user', toml: BLANK_PARSER }, { isNew: true });
          if (!library) return null;
          patchState(store, { library });
          return $localize`:@@settings.parsers.notice.created:Saved the new parser.`;
        }),

      /** Saves a parser TOML (the format wizard); rejects like the engine (e.g. code `fileExists` on an id clash). */
      async save(toml: string, options: ParserSaveOptions = {}): Promise<ParserLibraryDto> {
        const library = await backend.save(toml, options);
        patchState(store, { library });
        return library;
      },

      /** "Import .toml…": a new parser is added; one already in the library is replaced after the user saw the diff. */
      importFromFile: () =>
        act('*', async () => {
          const path = await files.pickImport();
          if (!path) return null;
          const check = await backend.importCheck(path);
          const { name, version } = check;
          if (check.relation === 'new') {
            patchState(store, { library: await backend.save(check.toml) });
            return $localize`:@@settings.parsers.notice.imported:Imported ${name}:name: ${version}:version:.`;
          }
          if (!(await dialogs.confirmImport(check))) return null;
          patchState(store, { library: await backend.save(check.toml, { previousId: check.id, overwrite: true }) });
          return check.relation === 'newer'
            ? $localize`:@@settings.parsers.notice.updated:Updated ${name}:name: to ${version}:version:.`
            : $localize`:@@settings.parsers.notice.replaced:Replaced ${name}:name: with version ${version}:version:.`;
        }),

      /** "Export…": the parser file as stored, to a path the user chose (asks before replacing a file). */
      exportParser: (id: string) =>
        act(id, async () => {
          const path = await files.pickExport(id);
          if (!path) return null;
          const done = await guard.write(path, async (overwrite) => {
            await backend.exportTo(id, path, overwrite);
            return true;
          });
          return done ? $localize`:@@settings.parsers.notice.exported:Exported ${id}:id: to ${path}:path:.` : null;
        }),

      /** Re-reads the window's sources whose parser changed; the workspace follows the job like an import. */
      reread: () =>
        act('*', async () => {
          const job = await backend.reread();
          await workspace.followReread(job);
          await load();
          return $localize`:@@settings.parsers.notice.reread:Read the files again with their parsers.`;
        }),

      /** "Parse with…": one source with that parser (null: the automatic choice). */
      assign: (sourceId: number, parserId: string | null) =>
        act('*', async () => {
          const job = await backend.assign(sourceId, parserId);
          await workspace.followReread(job);
          await load();
          return null;
        }),

      dismiss: () => patchState(store, { notice: null, actionError: null }),
    };
  }),
);
