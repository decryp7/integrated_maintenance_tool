import { SourceItem } from '../live/source-groups';

/** Characters of a file name kept visible at the end when it is shortened in the middle ("…-04.log"). */
export const NAME_TAIL = 11;

/** A name in two parts: the head, which may be cut with an ellipsis, and the tail, which always stays visible. */
export interface SplitName {
  head: string;
  tail: string;
}

/**
 * Splits a name so that a narrow row shows its beginning and its end ("extra-fo…-04.log") instead of the same prefix
 * for every sibling. Short names stay whole. Purely visual: the full path is the row's tooltip.
 */
export function splitMiddle(name: string, tail = NAME_TAIL): SplitName {
  if (name.length <= tail * 2) return { head: name, tail: '' };
  return { head: name.slice(0, name.length - tail), tail: name.slice(name.length - tail) };
}

/** The folder part of a path ("" when it has none). */
export function folderOf(path: string): string {
  const parts = path.split(/[\\/]/);
  parts.pop();
  return parts.join('/');
}

/** The last two segments of a folder, e.g. "logs/web01". */
export function folderLabel(folder: string): string {
  return folder.split('/').filter(Boolean).slice(-2).join('/') || folder || '/';
}

/** The path an explorer item stands for (a group: its newest file). */
function pathOf(item: SourceItem): string {
  return item.kind === 'file' ? item.source.path : item.group.newest.path;
}

/** An explorer item, with the folder heading to draw before it when it starts a new folder. */
export interface ExplorerRow {
  item: SourceItem;
  heading: { label: string; path: string } | null;
}

/** The segments of a folder, without empty ones. */
function segments(folder: string): string[] {
  return folder.split('/').filter(Boolean);
}

/** The leading folder segments every folder shares (joined as the first folder wrote them), "" when none. */
export function commonFolder(folders: readonly string[]): string {
  const first = folders[0];
  if (first === undefined) return '';
  const lead = first.startsWith('/') ? '/' : '';
  const shared = segments(first);
  let n = shared.length;
  for (const f of folders) {
    const parts = segments(f);
    let i = 0;
    while (i < n && i < parts.length && parts[i] === shared[i]) i++;
    n = i;
  }
  return n === 0 ? '' : lead + shared.slice(0, n).join('/');
}

/** A folder relative to the shared prefix ("web01"), or its own last segment when it is the prefix itself. */
export function folderRelative(folder: string, root: string): string {
  const rest = segments(folder).slice(segments(root).length);
  return rest.length ? rest.join('/') : (segments(folder).pop() ?? folderLabel(folder));
}

/** The explorer's rows, whether files are grouped under folder headings, and the folder prefix every heading shares. */
export interface ExplorerLayout {
  rows: ExplorerRow[];
  byFolder: boolean;
  /** The common folder of all listed paths, shown once above the headings (which are relative to it); "" when none. */
  root: string;
}

/**
 * Orders the explorer items by folder (first appearance first, stable inside a folder) and marks where each folder
 * starts, but only when more than one folder is open: one folder needs no heading. The prefix every folder shares
 * ("incident-2026-10-14") is hoisted into `root` so the headings only say what differs ("web01", "web02").
 */
export function explorerRows(items: readonly SourceItem[]): ExplorerLayout {
  const order: string[] = [];
  const byFolder = new Map<string, SourceItem[]>();
  for (const item of items) {
    const folder = folderOf(pathOf(item));
    const list = byFolder.get(folder);
    if (list) list.push(item);
    else {
      order.push(folder);
      byFolder.set(folder, [item]);
    }
  }
  if (order.length < 2) return { rows: items.map((item) => ({ item, heading: null })), byFolder: false, root: '' };
  const root = commonFolder(order);
  const label = (folder: string) => (root ? folderRelative(folder, root) : folderLabel(folder));
  const rows = order.flatMap((folder) => (byFolder.get(folder) ?? []).map((item, i): ExplorerRow => ({ item, heading: i === 0 ? { label: label(folder), path: folder } : null })));
  return { rows, byFolder: true, root };
}
