import { EMPTY_QUERY } from '../../core/workspace.store';
import { RecordView, SourceInfo } from '../../core/api.types';
import { DebriefBackend } from '../../core/backend';
import { REGISTRY_ROW_CAP } from './registry-tree';

const PAGE = 2000;

export interface RegistryRows {
  rows: RecordView[];
  /** True when more rows exist than were read. */
  truncated: boolean;
}

/** Reads all rows of the given registry sources (filtered by `@source`), page by page, up to the cap. */
export async function loadRegistryRows(backend: DebriefBackend, sources: readonly SourceInfo[], cap = REGISTRY_ROW_CAP): Promise<RegistryRows> {
  const rows: RecordView[] = [];
  let truncated = false;
  for (const source of sources) {
    const query = { ...EMPTY_QUERY, filters: [{ field: '@source', op: 'eq' as const, value: source.name }] };
    let offset = 0;
    for (;;) {
      const limit = Math.min(PAGE, cap - rows.length);
      if (limit <= 0) {
        truncated = true;
        break;
      }
      const page = await backend.queryPage({ query, offset, limit, descending: false, anchor: null });
      // Another source may share the file name: keep only rows of this source.
      rows.push(...page.rows.filter((r) => r.source === source.id));
      offset += page.rows.length;
      if (!page.rows.length || offset >= page.total) break;
    }
    if (truncated) break;
  }
  return { rows, truncated };
}
