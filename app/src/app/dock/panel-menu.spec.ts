import { PanelFacts, panelMenu, panelsToClose, pinnedIndex } from './panel-menu';

const facts = (over: Partial<PanelFacts> = {}): PanelFacts => ({
  kind: 'tool',
  floating: false,
  hasSiblings: true,
  hasRight: true,
  groups: 2,
  pinnedTab: false,
  canPopout: true,
  inDocumentWell: false,
  ...over,
});
const find = (items: ReturnType<typeof panelMenu>, action: string) => items.find((i) => i.action === action);
const actions = (f: PanelFacts) => panelMenu(f).map((i) => i.action);

describe('panelMenu', () => {
  it('offers window positions, auto hide and close for a tool window', () => {
    expect(actions(facts())).toEqual(['float', 'popout', 'dock', 'dockAsDocument', 'autoHide', 'pinTab', 'close']);
  });

  it('offers the tab-group commands for a document, but no auto hide', () => {
    expect(actions(facts({ kind: 'document' }))).toEqual([
      'float',
      'popout',
      'dock',
      'pinTab',
      'close',
      'closeOthers',
      'closeRight',
      'closeAllDocuments',
      'newHorizontalGroup',
      'newVerticalGroup',
      'moveToNextGroup',
    ]);
  });

  it('docked panels can float but not dock; floating panels can dock but not float', () => {
    expect(find(panelMenu(facts({ floating: false })), 'float')?.disabled).toBe(false);
    expect(find(panelMenu(facts({ floating: false })), 'dock')?.disabled).toBe(true);
    expect(find(panelMenu(facts({ floating: true })), 'float')?.disabled).toBe(true);
    expect(find(panelMenu(facts({ floating: true })), 'dock')?.disabled).toBe(false);
  });

  it('disables pop out when it is not possible', () => {
    expect(find(panelMenu(facts({ canPopout: false })), 'popout')?.disabled).toBe(true);
    expect(find(panelMenu(facts()), 'popout')?.disabled).toBe(false);
  });

  it('disables dock-as-document only for docked tools already in the document well', () => {
    expect(find(panelMenu(facts({ inDocumentWell: true })), 'dockAsDocument')?.disabled).toBe(true);
    expect(find(panelMenu(facts({ inDocumentWell: true, floating: true })), 'dockAsDocument')?.disabled).toBe(false);
    expect(find(panelMenu(facts()), 'dockAsDocument')?.disabled).toBe(false);
  });

  it('toggles between pin and unpin tab', () => {
    expect(find(panelMenu(facts()), 'pinTab')?.label).toBe('Pin tab');
    expect(find(panelMenu(facts({ pinnedTab: true })), 'pinTab')).toBeUndefined();
    expect(find(panelMenu(facts({ pinnedTab: true })), 'unpinTab')?.label).toBe('Unpin tab');
  });

  it('disables document commands that have nothing to act on', () => {
    const none = panelMenu(facts({ kind: 'document', hasSiblings: false, hasRight: false, groups: 1 }));
    for (const a of ['closeOthers', 'closeRight', 'newHorizontalGroup', 'newVerticalGroup', 'moveToNextGroup']) expect(find(none, a)?.disabled).toBe(true);
    expect(find(none, 'closeAllDocuments')?.disabled).toBeUndefined();
    expect(find(none, 'close')?.disabled).toBeUndefined();

    const some = panelMenu(facts({ kind: 'document' }));
    for (const a of ['closeOthers', 'closeRight', 'newHorizontalGroup', 'newVerticalGroup', 'moveToNextGroup']) expect(find(some, a)?.disabled).toBe(false);
  });

  it('disables group moves for floating documents', () => {
    const floating = panelMenu(facts({ kind: 'document', floating: true }));
    for (const a of ['newHorizontalGroup', 'newVerticalGroup', 'moveToNextGroup']) expect(find(floating, a)?.disabled).toBe(true);
  });
});

describe('panelsToClose', () => {
  const order = ['a', 'b', 'c', 'd'];
  const none = new Set<string>();

  it('closes all others', () => {
    expect(panelsToClose(order, 'b', 'others', none)).toEqual(['a', 'c', 'd']);
  });

  it('closes everything to the right', () => {
    expect(panelsToClose(order, 'b', 'right', none)).toEqual(['c', 'd']);
    expect(panelsToClose(order, 'd', 'right', none)).toEqual([]);
  });

  it('closes everything for all, including the target', () => {
    expect(panelsToClose(order, 'b', 'all', none)).toEqual(order);
  });

  it('never closes pinned tabs', () => {
    const pinned = new Set(['a', 'c']);
    expect(panelsToClose(order, 'b', 'others', pinned)).toEqual(['d']);
    expect(panelsToClose(order, 'a', 'right', pinned)).toEqual(['b', 'd']);
    expect(panelsToClose(order, 'b', 'all', pinned)).toEqual(['b', 'd']);
  });

  it('closes nothing to the right of an unknown target', () => {
    expect(panelsToClose(order, 'zzz', 'right', none)).toEqual([]);
  });
});

describe('pinnedIndex', () => {
  const order = ['a', 'b', 'c', 'd'];

  it('counts the pinned tabs other than the one being pinned', () => {
    expect(pinnedIndex(order, 'd', new Set(['a', 'b']))).toBe(2);
    expect(pinnedIndex(order, 'c', new Set())).toBe(0);
  });

  it('does not count the tab itself when it is already pinned', () => {
    expect(pinnedIndex(order, 'b', new Set(['a', 'b']))).toBe(1);
  });

  it('ignores pinned ids that are not in the strip', () => {
    expect(pinnedIndex(order, 'd', new Set(['gone']))).toBe(0);
  });
});
