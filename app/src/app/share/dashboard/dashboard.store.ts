import { computed, inject, Injectable, signal } from '@angular/core';
import { DashboardCfg, TileCfg } from '../../core/api.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { ProfileEdits } from '../../profile/profile-edits';
import { UndoStack } from '../../undo/undo-stack';
import { addTile, moveTile, placeTile, removeTile, resizeTile, sizeTile, uniqueId } from './dashboard-layout';

/** The profile's `[[dashboard]]` as written in TOML: only the keys that are set. */
export function toProfileDashboard(d: DashboardCfg): Record<string, unknown> {
  return {
    id: d.id,
    title: d.title,
    columns: d.columns,
    tile: d.tile.map((t) => {
      const out: Record<string, unknown> = {};
      for (const [k, v] of Object.entries(t)) {
        if (v === undefined || v === null || (typeof v === 'string' && v === '' && k !== 'title') || (Array.isArray(v) && !v.length)) continue;
        out[k] = v;
      }
      return out;
    }),
  };
}

/** Undo group of the layout edits of the dashboard being edited. */
const DRAFT_GROUP = 'dashboard-draft';

/**
 * Which dashboard is shown, and the edit session: edits change a draft that only becomes the profile's
 * `[[dashboard]]` on Save (the engine validates it; its message is shown when it refuses).
 */
@Injectable({ providedIn: 'root' })
export class DashboardStore {
  private readonly workspace = inject(WorkspaceStore);
  private readonly edits = inject(ProfileEdits);
  private readonly undo = inject(UndoStack);

  readonly dashboards = computed<DashboardCfg[]>(() => this.workspace.summary()?.dashboards ?? []);
  readonly selectedId = signal<string | null>(null);
  readonly draft = signal<DashboardCfg | null>(null);
  readonly editing = computed(() => this.draft() !== null);
  readonly saving = signal(false);
  readonly error = signal<string | null>(null);
  readonly wall = signal(false);
  /** The last layout change, for screen readers. */
  readonly announcement = signal('');
  /** The draft is a dashboard that is not in the profile yet. */
  private readonly isNew = signal(false);

  /** The dashboard on screen: the draft while editing, else the selected (or first) one. */
  readonly current = computed<DashboardCfg | null>(() => {
    const draft = this.draft();
    if (draft) return draft;
    const all = this.dashboards();
    return all.find((d) => d.id === this.selectedId()) ?? all[0] ?? null;
  });

  select(id: string): void {
    if (this.editing()) return;
    this.selectedId.set(id);
  }

  startEdit(): void {
    const d = this.current();
    if (!d || this.editing()) return;
    this.isNew.set(false);
    this.draft.set(structuredClone(d));
    this.error.set(null);
  }

  /** Starts a new, empty dashboard in edit mode. */
  newDashboard(title = $localize`:@@share.dashboardStore.newTitle:New dashboard`): void {
    const id = uniqueId(this.dashboards().map((d) => d.id), title);
    this.isNew.set(true);
    this.draft.set({ id, title, columns: 12, tile: [] });
    this.error.set(null);
  }

  cancel(): void {
    this.draft.set(null);
    this.error.set(null);
    this.undo.discard(DRAFT_GROUP);
  }

  setTitle(title: string): void {
    this.update((d) => ({ ...d, title }), $localize`:@@share.dashboardStore.undoTitle:dashboard title edit`, 'dashboard.title');
  }

  /** Changes the draft; undoable while the edit lasts (the entries are dropped on Save or Cancel). */
  private update(change: (d: DashboardCfg) => DashboardCfg, label = $localize`:@@share.dashboardStore.undoLayout:dashboard layout change`, mergeKey?: string): void {
    const before = this.draft();
    if (!before) return;
    const after = change(before);
    this.draft.set(after);
    if (JSON.stringify(before) === JSON.stringify(after)) return;
    const put = (d: DashboardCfg) => {
      if (this.draft()) this.draft.set(d);
    };
    this.undo.record({ label, mergeKey, group: DRAFT_GROUP, do: () => put(after), undo: () => put(before) });
  }

  private tiles(change: (tiles: TileCfg[], columns: number) => TileCfg[], said: (t: TileCfg | undefined) => string, id: string, label?: string): void {
    this.update((d) => {
      const tile = change(d.tile, d.columns);
      this.announcement.set(said(tile.find((t) => t.id === id)));
      return { ...d, tile };
    }, label);
  }

  private static where(t: TileCfg | undefined): string {
    return t ? $localize`:@@share.dashboardStore.where:${t.title || t.id}:title: column ${t.x + 1}:column:, row ${t.y + 1}:row:, ${t.w}:w: by ${t.h}:h:` : '';
  }

  move(id: string, dx: number, dy: number): void {
    this.tiles((tiles, cols) => moveTile(tiles, id, dx, dy, cols), DashboardStore.where, id);
  }

  place(id: string, x: number, y: number): void {
    this.tiles((tiles, cols) => placeTile(tiles, id, x, y, cols), DashboardStore.where, id);
  }

  resize(id: string, dw: number, dh: number): void {
    this.tiles((tiles, cols) => resizeTile(tiles, id, dw, dh, cols), DashboardStore.where, id);
  }

  setSize(id: string, w: number, h: number): void {
    this.tiles((tiles, cols) => sizeTile(tiles, id, w, h, cols), DashboardStore.where, id);
  }

  remove(id: string): void {
    const t = this.draft()?.tile.find((x) => x.id === id);
    this.tiles((tiles) => removeTile(tiles, id), () => (t ? $localize`:@@share.dashboardStore.removed:${t.title || t.id}:title: removed` : ''), id, $localize`:@@share.dashboardStore.undoRemove:dashboard tile removal`);
  }

  /** Adds a tile (a fresh id when its id is taken); returns the id. */
  add(tile: TileCfg): string | null {
    const d = this.draft();
    if (!d) return null;
    const id = uniqueId(d.tile.map((t) => t.id), tile.id || tile.title || tile.type);
    this.tiles((tiles, cols) => addTile(tiles, { ...tile, id }, cols), (t) => $localize`:@@share.dashboardStore.added:${t?.title || id}:title: added: ${DashboardStore.where(t)}:where:`, id, $localize`:@@share.dashboardStore.undoAdd:dashboard tile addition`);
    return id;
  }

  /** Writes the draft into the profile (re-validated by the engine); stays in edit mode when refused. */
  async save(): Promise<boolean> {
    const d = this.draft();
    if (!d) return false;
    this.saving.set(true);
    this.error.set(null);
    const isNew = this.isNew();
    try {
      await this.edits.apply((model) => {
        const list = Array.isArray(model['dashboard']) ? (model['dashboard'] as Record<string, unknown>[]) : [];
        const entry = toProfileDashboard(d);
        const at = list.findIndex((x) => x['id'] === d.id);
        model['dashboard'] = at >= 0 && !isNew ? list.map((x, i) => (i === at ? entry : x)) : [...list, entry];
      }, $localize`:@@share.dashboardStore.undoSave:dashboard save`);
      this.selectedId.set(d.id);
      this.draft.set(null);
      this.undo.discard(DRAFT_GROUP);
      this.announcement.set($localize`:@@share.dashboardStore.saved:Dashboard ${d.title}:title: saved`);
      return true;
    } catch (e) {
      this.error.set(e instanceof Error ? e.message : String(e));
      return false;
    } finally {
      this.saving.set(false);
    }
  }
}
