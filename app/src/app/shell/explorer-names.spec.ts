import { SourceInfo } from '../core/api.types';
import { SourceItem } from '../live/source-groups';
import { commonFolder, explorerRows, folderLabel, folderRelative, folderOf, NAME_TAIL, splitMiddle } from './explorer-names';

const file = (path: string, id = 1): SourceItem => ({ kind: 'file', source: { id, path, name: path.split('/').pop() } as SourceInfo });

describe('splitMiddle', () => {
  it('keeps short names whole and cuts long ones so that their end stays visible (H2)', () => {
    expect(splitMiddle('app.log')).toEqual({ head: 'app.log', tail: '' });
    const parts = splitMiddle('extra-forensic-collection-04.log');
    expect(parts.tail).toBe('tion-04.log');
    expect(parts.head + parts.tail).toBe('extra-forensic-collection-04.log');
    expect(splitMiddle('a'.repeat(NAME_TAIL * 2)).tail).toBe('');
    expect(splitMiddle('a'.repeat(NAME_TAIL * 2 + 1)).tail).toHaveLength(NAME_TAIL);
  });

  it('keeps siblings with the same prefix apart', () => {
    const tails = ['extra-forensic-collection-03.log', 'extra-forensic-collection-04.log'].map((n) => splitMiddle(n).tail);
    expect(new Set(tails).size).toBe(2);
  });
});

describe('folders', () => {
  it('reads the folder of a path and labels it with its last two segments', () => {
    expect(folderOf('/logs/web01/a.log')).toBe('/logs/web01');
    expect(folderOf('C:\\logs\\web01\\a.log')).toBe('C:/logs/web01');
    expect(folderOf('a.log')).toBe('');
    expect(folderLabel('/logs/web01')).toBe('logs/web01');
    expect(folderLabel('')).toBe('/');
  });
});

describe('explorerRows', () => {
  it('adds no headings for one folder', () => {
    const r = explorerRows([file('/logs/a.log', 1), file('/logs/b.log', 2)]);
    expect(r.byFolder).toBe(false);
    expect(r.rows.every((x) => x.heading === null)).toBe(true);
  });

  it('groups by folder in order of first appearance, with a heading where each folder starts', () => {
    const r = explorerRows([file('/logs/web01/a.log', 1), file('/logs/web02/b.log', 2), file('/logs/web01/c.log', 3)]);
    expect(r.byFolder).toBe(true);
    expect(r.rows.map((x) => (x.item.kind === 'file' ? x.item.source.name : ''))).toEqual(['a.log', 'c.log', 'b.log']);
    expect(r.rows.map((x) => x.heading?.label ?? null)).toEqual(['web01', null, 'web02']);
    expect(r.root).toBe('/logs');
  });

  it('places a rolling-file group by its newest file', () => {
    const newest = { id: 9, path: '/logs/web02/app.log', name: 'app.log' } as SourceInfo;
    const group: SourceItem = { kind: 'group', group: { key: 'g', label: 'app.log', sources: [newest], records: 1, newest, split: false } } as SourceItem;
    const r = explorerRows([file('/logs/web01/a.log', 1), group]);
    expect(r.rows.map((x) => x.heading?.label)).toEqual(['web01', 'web02']);
  });
});

describe('folder prefix hoisting (U14)', () => {
  it('finds the segments every folder shares', () => {
    expect(commonFolder([])).toBe('');
    expect(commonFolder(['/a/b/c', '/a/b/d'])).toBe('/a/b');
    expect(commonFolder(['/a/b', '/a/b/d'])).toBe('/a/b');
    expect(commonFolder(['/a/b', '/x/y'])).toBe('');
    expect(commonFolder(['a/b', 'a/c'])).toBe('a');
    expect(commonFolder(['/same', '/same'])).toBe('/same');
  });

  it('labels a folder relative to the shared prefix, or by its own name when it is the prefix', () => {
    expect(folderRelative('/inc/web01/logs', '/inc')).toBe('web01/logs');
    expect(folderRelative('/inc', '/inc')).toBe('inc');
    expect(folderRelative('/', '/')).toBe('/');
  });

  it('hoists the shared prefix of long incident folders and keeps unrelated folders whole', () => {
    const r = explorerRows([file('/inc-2026-10-14/web01/a.log', 1), file('/inc-2026-10-14/web02/b.log', 2), file('/inc-2026-10-14/a.log', 3)]);
    expect(r.root).toBe('/inc-2026-10-14');
    expect(r.rows.map((x) => x.heading?.label ?? null)).toEqual(['web01', 'web02', 'inc-2026-10-14']);
    const apart = explorerRows([file('/a/x/a.log', 1), file('/b/y/b.log', 2)]);
    expect(apart.root).toBe('');
    expect(apart.rows.map((x) => x.heading?.label)).toEqual(['a/x', 'b/y']);
  });
});
