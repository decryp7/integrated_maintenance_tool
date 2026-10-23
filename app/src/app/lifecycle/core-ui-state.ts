import { inject, Injectable } from '@angular/core';
import { Query } from '../core/api.types';
import { EMPTY_QUERY, WorkspaceStore } from '../core/workspace.store';
import { DockService } from '../dock/dock.service';
import { LAYOUT_VERSION } from '../dock/layout-store';
import { ViewRegistry } from '../extensions/view-registry';
import { UiStateContribution } from './ui-state';

const QUERY_KEYS = Object.keys(EMPTY_QUERY) as (keyof Query)[];

/** Keeps only known query keys of untrusted saved state, on top of the empty query. */
export function sanitizeQuery(value: unknown): Query {
  const given = value && typeof value === 'object' ? (value as Record<string, unknown>) : {};
  const query: Record<string, unknown> = { ...EMPTY_QUERY };
  for (const k of QUERY_KEYS) if (k in given) query[k] = given[k];
  return query as unknown as Query;
}

/** The UI state every session has: the query, the layout (open views) and the selection. */
@Injectable({ providedIn: 'root' })
export class CoreUiState {
  private readonly store = inject(WorkspaceStore);
  private readonly dock = inject(DockService);
  private readonly views = inject(ViewRegistry);

  readonly contributions: UiStateContribution[] = [
    {
      key: 'query',
      // The search box text counts as applied even while its debounce is pending, so saving right after typing is consistent.
      // How the text matches (`textMode`) is left out: smart search derives it from the text after a debounce and a
      // background scan, so it is no user change (it made the unsaved dot appear right after a save) and restoring the
      // text derives it again.
      capture: () => ({ query: { ...this.store.query(), text: this.store.textInput(), textMode: undefined }, textInput: this.store.textInput() }),
      restore: (v) => {
        const saved = (v ?? {}) as { query?: unknown; textInput?: unknown };
        const query = sanitizeQuery(saved.query);
        this.store.patchQuery(query);
        this.store.setTextInput(typeof saved.textInput === 'string' ? saved.textInput : query.text);
      },
      reset: () => this.store.clearQuery(),
    },
    {
      key: 'layout',
      tracksChanges: false,
      capture: () => ({ id: `debrief.layout.v${LAYOUT_VERSION}`, openViews: [...this.dock.openIds()].sort() }),
      restore: (v) => {
        const open = (v as { openViews?: unknown } | null)?.openViews;
        if (!Array.isArray(open)) return;
        for (const id of open) if (typeof id === 'string' && this.views.find(id) && !this.dock.isOpen(id)) this.dock.show(id, { inactive: true });
      },
    },
    {
      key: 'selection',
      tracksChanges: false,
      capture: () => ({ recordId: this.store.selected()?.id ?? null }),
      restore: (v) => {
        const id = (v as { recordId?: unknown } | null)?.recordId;
        if (typeof id === 'number') this.store.jumpTo(id);
      },
    },
  ];
}
