import { ViewKind } from './view-kind';

/** What the panel menu (a tab's ▾ and context menu, VS's "Window Position" menu) can do. */
export type PanelAction =
  | 'float'
  | 'popout'
  | 'dock'
  | 'dockAsDocument'
  | 'autoHide'
  | 'close'
  | 'closeOthers'
  | 'closeRight'
  | 'closeAllDocuments'
  | 'newHorizontalGroup'
  | 'newVerticalGroup'
  | 'moveToNextGroup'
  | 'pinTab'
  | 'unpinTab';

export interface PanelMenuItem {
  action: PanelAction;
  label: string;
  disabled?: boolean;
}

/** The facts about a panel that decide its menu. */
export interface PanelFacts {
  kind: ViewKind;
  floating: boolean;
  /** The panel shares its group with other tabs. */
  hasSiblings: boolean;
  /** There are tabs to its right in its group. */
  hasRight: boolean;
  /** Number of groups in the grid (for "Move to next tab group"). */
  groups: number;
  pinnedTab: boolean;
  /** Pop-out windows are possible (and the limit is not reached). */
  canPopout: boolean;
  /** It already sits in a group with documents. */
  inDocumentWell: boolean;
}

/** The menu for one panel, in VS's order. */
export function panelMenu(f: PanelFacts): PanelMenuItem[] {
  const items: PanelMenuItem[] = [
    { action: 'float', label: 'Float inside window', disabled: f.floating },
    { action: 'popout', label: 'Pop out to window', disabled: !f.canPopout },
    { action: 'dock', label: 'Dock', disabled: !f.floating },
  ];
  if (f.kind === 'tool') {
    items.push({ action: 'dockAsDocument', label: 'Dock as tabbed document', disabled: f.inDocumentWell && !f.floating });
    items.push({ action: 'autoHide', label: 'Auto hide' });
  }
  items.push({ action: f.pinnedTab ? 'unpinTab' : 'pinTab', label: f.pinnedTab ? 'Unpin tab' : 'Pin tab' });
  items.push({ action: 'close', label: 'Close' });
  if (f.kind === 'document') {
    items.push(
      { action: 'closeOthers', label: 'Close all but this', disabled: !f.hasSiblings },
      { action: 'closeRight', label: 'Close all to the right', disabled: !f.hasRight },
      { action: 'closeAllDocuments', label: 'Close all documents' },
      { action: 'newHorizontalGroup', label: 'New horizontal tab group', disabled: !f.hasSiblings || f.floating },
      { action: 'newVerticalGroup', label: 'New vertical tab group', disabled: !f.hasSiblings || f.floating },
      { action: 'moveToNextGroup', label: 'Move to next tab group', disabled: f.groups < 2 || f.floating },
    );
  }
  return items;
}

/** Which panels "close all but this" / "to the right" / "all" close in one tab strip: never pinned tabs. */
export function panelsToClose(order: readonly string[], target: string, scope: 'others' | 'right' | 'all', pinned: ReadonlySet<string>): string[] {
  const at = order.indexOf(target);
  return order.filter((id, i) => {
    if (pinned.has(id)) return false;
    if (scope === 'others') return id !== target;
    if (scope === 'right') return at >= 0 && i > at;
    return true;
  });
}

/** Where a tab goes when it is pinned: right after the pinned tabs already at the left of its strip. */
export function pinnedIndex(order: readonly string[], id: string, pinned: ReadonlySet<string>): number {
  return order.filter((p) => p !== id && pinned.has(p)).length;
}
