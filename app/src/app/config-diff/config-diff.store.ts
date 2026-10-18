import { OverwriteGuard } from '../ui/overwrite-guard';
import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import {
  CHANGE_KINDS,
  ChangeKind,
  DiffCounts,
  DiffExportFormat,
  DiffOptions,
  DiffRow,
  ExpandMode,
  FormatInfo,
  SideInfo,
  SnapshotDiff,
  SnapshotFile,
} from '../core/config-diff.types';
import { copyText } from '../tools/clipboard';
import { errorMessage } from '../tools/tool-errors';
import { plural } from '../i18n/locale';
import { ConfigDiffBackend } from './config-diff-backend';
import { ConfigDiffPicker, Side } from './config-diff-picker';

/** "Copied path" / "Copied old value" / ...: one full message per label the view uses. */
function copiedMessage(label: string): string {
  if (label === 'path') return $localize`:@@configDiff.store.copied.path:Copied path`;
  if (label === 'old value') return $localize`:@@configDiff.store.copied.oldValue:Copied old value`;
  if (label === 'new value') return $localize`:@@configDiff.store.copied.newValue:Copied new value`;
  return $localize`:@@configDiff.store.copied.other:Copied ${label}:label:`;
}

function exportedMessage(rows: number, path: string): string {
  return plural(rows, {
    one: $localize`:@@configDiff.store.exported.one:Exported ${rows}:count: row to ${path}:path:`,
    other: $localize`:@@configDiff.store.exported.other:Exported ${rows}:count: rows to ${path}:path:`,
  });
}

/** Rows fetched per request. */
export const PAGE_SIZE = 200;

export type DiffLayout = 'split' | 'unified';
export type DiffMode = 'files' | 'folders';

/** What the engine reported about the current diff (everything but the rows). */
export interface DiffMeta {
  left: SideInfo;
  right: SideInfo;
  counts: DiffCounts;
  total: number;
  truncated: boolean;
  caseInsensitive: boolean;
}

interface ConfigDiffState {
  mode: DiffMode;
  left: string;
  right: string;
  /** Ignore patterns; null until the engine's defaults are loaded. */
  ignore: string[] | null;
  defaultIgnore: string[];
  caseInsensitive: boolean;
  kinds: ChangeKind[];
  search: string;
  expand: ExpandMode;
  toggled: string[];
  layout: DiffLayout;
  formats: FormatInfo[];
  meta: DiffMeta | null;
  /** Fetched pages by page number; cleared whenever the view changes. */
  pages: Record<number, DiffRow[]>;
  /** Bumped on every view change; answers for older generations are dropped. */
  generation: number;
  loading: boolean;
  error: string | null;
  notice: string | null;
  selected: number | null;
  leftDir: string;
  rightDir: string;
  snapshot: SnapshotDiff | null;
  snapshotBusy: boolean;
  snapshotError: string | null;
}

const INITIAL: ConfigDiffState = {
  mode: 'files',
  left: '',
  right: '',
  ignore: null,
  defaultIgnore: [],
  caseInsensitive: false,
  kinds: [...CHANGE_KINDS],
  search: '',
  expand: 'all',
  toggled: [],
  layout: 'split',
  formats: [],
  meta: null,
  pages: {},
  generation: 0,
  loading: false,
  error: null,
  notice: null,
  selected: null,
  leftDir: '',
  rightDir: '',
  snapshot: null,
  snapshotBusy: false,
  snapshotError: null,
};

/** One pattern per line, blank lines dropped. */
export function parsePatterns(text: string): string[] {
  return text
    .split('\n')
    .map((l) => l.trim())
    .filter(Boolean);
}

/** State of the configuration diff view: files, options, the paged rows and the snapshot comparison. */
export const ConfigDiffStore = signalStore(
  { providedIn: 'root' },
  withState<ConfigDiffState>(INITIAL),
  withComputed((store) => ({
    ready: computed(() => !!store.left().trim() && !!store.right().trim()),
    total: computed(() => store.meta()?.total ?? 0),
    ignoreText: computed(() => (store.ignore() ?? store.defaultIgnore()).join('\n')),
    selectedRow: computed(() => {
      const i = store.selected();
      return i === null ? null : (store.pages()[Math.floor(i / PAGE_SIZE)]?.[i % PAGE_SIZE] ?? null);
    }),
  })),
  withMethods((store, backend = inject(ConfigDiffBackend), picker = inject(ConfigDiffPicker), overwrite = inject(OverwriteGuard)) => {
    const inFlight = new Set<string>();

    const options = (offset: number): DiffOptions => ({
      compare: { ignore: store.ignore() ?? store.defaultIgnore(), caseInsensitive: store.caseInsensitive() },
      view: { kinds: store.kinds(), search: store.search(), expand: store.expand(), toggled: store.toggled(), offset, limit: PAGE_SIZE },
    });

    const fetchPage = async (page: number): Promise<void> => {
      const generation = store.generation();
      const key = `${generation}:${page}`;
      if (!store.ready() || store.pages()[page] || inFlight.has(key)) return;
      inFlight.add(key);
      patchState(store, { loading: true });
      try {
        const result = await backend.diff(store.left().trim(), store.right().trim(), options(page * PAGE_SIZE));
        if (generation !== store.generation()) return;
        const meta: DiffMeta = {
          left: result.left,
          right: result.right,
          counts: result.counts,
          total: result.total,
          truncated: result.truncated,
          caseInsensitive: result.caseInsensitive,
        };
        patchState(store, (s) => ({ meta, pages: { ...s.pages, [page]: result.rows }, error: null }));
      } catch (e) {
        if (generation === store.generation()) patchState(store, { error: errorMessage(e), meta: null, pages: {} });
      } finally {
        inFlight.delete(key);
        if (generation === store.generation()) patchState(store, { loading: false });
      }
    };

    /** Drops fetched rows (the view changed) and loads the first page again. */
    const refresh = async (keepSelection = false): Promise<void> => {
      patchState(store, (s) => ({ pages: {}, generation: s.generation + 1, selected: keepSelection ? s.selected : null, notice: null }));
      await fetchPage(0);
    };

    const compareFolders = async (): Promise<void> => {
      patchState(store, { snapshotBusy: true, snapshotError: null });
      try {
        const ignore = store.ignore() ?? store.defaultIgnore();
        const snapshot = await backend.snapshots(store.leftDir().trim(), store.rightDir().trim(), { ignore, caseInsensitive: store.caseInsensitive() });
        patchState(store, { snapshot });
      } catch (e) {
        patchState(store, { snapshotError: errorMessage(e), snapshot: null });
      } finally {
        patchState(store, { snapshotBusy: false });
      }
    };

    return {
      /** Loads the supported formats and the default ignore patterns. Never rejects. */
      async init(): Promise<void> {
        try {
          const f = await backend.formats();
          patchState(store, { formats: f.formats, defaultIgnore: f.defaultIgnore });
        } catch (e) {
          patchState(store, { error: errorMessage(e) });
        }
      },
      setMode: (mode: DiffMode) => patchState(store, { mode }),
      setLayout: (layout: DiffLayout) => patchState(store, { layout }),
      setPath: (side: Side, path: string) => patchState(store, side === 'left' ? { left: path } : { right: path }),
      /** Asks for a file; compares when both sides are chosen. */
      async choose(side: Side): Promise<void> {
        const path = await picker.pickFile(side);
        if (path === null) return;
        patchState(store, side === 'left' ? { left: path } : { right: path });
        if (store.ready()) await refresh();
      },
      async swap(): Promise<void> {
        patchState(store, (s) => ({ left: s.right, right: s.left }));
        if (store.ready()) await refresh();
      },
      compare: () => refresh(),
      /** Fetches the pages covering rows `first`..`last` (exclusive) that are missing. */
      async ensure(first: number, last: number): Promise<void> {
        if (!store.meta() || last <= first) return;
        const pages = [];
        for (let p = Math.floor(first / PAGE_SIZE); p <= Math.floor((last - 1) / PAGE_SIZE); p++) pages.push(fetchPage(p));
        await Promise.all(pages);
      },
      /** The row at a position, when its page is loaded. */
      rowAt: (index: number): DiffRow | null => store.pages()[Math.floor(index / PAGE_SIZE)]?.[index % PAGE_SIZE] ?? null,
      async toggleKind(kind: ChangeKind): Promise<void> {
        patchState(store, (s) => ({ kinds: s.kinds.includes(kind) ? s.kinds.filter((k) => k !== kind) : CHANGE_KINDS.filter((k) => k === kind || s.kinds.includes(k)) }));
        if (store.ready()) await refresh();
      },
      async setSearch(search: string): Promise<void> {
        if (search === store.search()) return;
        patchState(store, { search });
        if (store.ready()) await refresh();
      },
      async setExpand(expand: ExpandMode): Promise<void> {
        patchState(store, { expand, toggled: [] });
        if (store.ready()) await refresh();
      },
      /** Expands or collapses a section. */
      async toggle(path: string): Promise<void> {
        patchState(store, (s) => ({ toggled: s.toggled.includes(path) ? s.toggled.filter((p) => p !== path) : [...s.toggled, path] }));
        await refresh(true);
      },
      async setIgnoreText(text: string): Promise<void> {
        patchState(store, { ignore: parsePatterns(text) });
        if (store.ready()) await refresh();
      },
      async resetIgnore(): Promise<void> {
        patchState(store, { ignore: null });
        if (store.ready()) await refresh();
      },
      async setCaseInsensitive(caseInsensitive: boolean): Promise<void> {
        patchState(store, { caseInsensitive });
        if (store.ready()) await refresh();
      },
      select: (index: number | null) => patchState(store, { selected: index }),
      /** Copies text and says so (or why not). */
      async copy(label: string, text: string): Promise<void> {
        const ok = await copyText(text);
        patchState(store, { notice: ok ? copiedMessage(label) : $localize`:@@configDiff.store.clipboardUnavailable:The clipboard is not available` });
      },
      dismiss: () => patchState(store, { error: null, notice: null }),
      /** Exports the diff (current kinds and search; all sections expanded). */
      async exportAs(format: DiffExportFormat): Promise<void> {
        const path = await picker.pickExport(format);
        if (path === null) return;
        try {
          const done = await overwrite.write(path, (replace) => backend.exportDiff(store.left().trim(), store.right().trim(), { ...options(0), view: { ...options(0).view, limit: 1 } }, format, path, replace));
          if (done === null) return;
          patchState(store, { notice: exportedMessage(done.rows, done.path), error: null });
        } catch (e) {
          patchState(store, { error: $localize`:@@configDiff.store.exportFailed:Could not export the diff: ${errorMessage(e)}:error:` });
        }
      },
      /** Asks for a snapshot folder; compares when both are chosen. */
      async chooseFolder(side: Side): Promise<void> {
        const dir = await picker.pickFolder(side);
        if (dir === null) return;
        patchState(store, side === 'left' ? { leftDir: dir } : { rightDir: dir });
        if (store.leftDir() && store.rightDir()) await compareFolders();
      },
      setDir: (side: Side, dir: string) => patchState(store, side === 'left' ? { leftDir: dir } : { rightDir: dir }),
      compareFolders,
      /** Shows the diff of one file pair of a snapshot comparison. */
      async openFile(file: SnapshotFile): Promise<void> {
        if (!file.leftPath || !file.rightPath) return;
        patchState(store, { mode: 'files', left: file.leftPath, right: file.rightPath, toggled: [] });
        await refresh();
      },
    };
  }),
);
