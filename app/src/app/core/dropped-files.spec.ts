import { describe, expect, it } from 'vitest';
import { DroppedFilesHandler, DropTargets, routeDrop, routeDroppedFiles } from './dropped-files';

describe('routeDroppedFiles', () => {
  it('leaves the paths no handler claims, in order', async () => {
    const captures: DroppedFilesHandler = async (paths) => paths.filter((p) => p.endsWith('.nettrace'));
    const heaps: DroppedFilesHandler = async (paths) => paths.filter((p) => p.endsWith('.heapsnapshot'));
    expect(await routeDroppedFiles([captures, heaps], ['/a.log', '/b.nettrace', '/c.heapsnapshot', '/d.log'])).toEqual(['/a.log', '/d.log']);
  });

  it('offers later handlers only what is left, and stops when everything is claimed', async () => {
    const seen: string[][] = [];
    const all: DroppedFilesHandler = async (paths) => (seen.push([...paths]), paths);
    const never: DroppedFilesHandler = async (paths) => (seen.push([...paths]), []);
    expect(await routeDroppedFiles([all, never], ['/x'])).toEqual([]);
    expect(seen).toEqual([['/x']]);
    expect(await routeDroppedFiles([], ['/y'])).toEqual(['/y']);
  });

  it('treats a failing handler as claiming nothing, so the files still open as logs', async () => {
    const broken: DroppedFilesHandler = async () => {
      throw new Error('sniff failed');
    };
    expect(await routeDroppedFiles([broken], ['/a.nettrace'])).toEqual(['/a.nettrace']);
  });
});

describe('drop targets', () => {
  function page() {
    const outer = document.createElement('div');
    const inner = document.createElement('span');
    outer.appendChild(inner);
    const other = document.createElement('p');
    return { outer, inner, other };
  }

  it('finds the innermost registered target under the point and stops after unregistering', () => {
    const { outer, inner, other } = page();
    const targets = new DropTargets();
    const accept: DroppedFilesHandler = async () => [];
    const offOuter = targets.register({ kind: 'aiImages', element: outer, accept });
    const innerTarget = { kind: 'aiImages' as const, element: inner, accept };
    const offInner = targets.register(innerTarget);
    let hit: Element | null = inner;
    const doc = { elementFromPoint: () => hit };
    expect(targets.at({ x: 1, y: 2 }, doc)).toBe(innerTarget);
    offInner();
    expect(targets.at({ x: 1, y: 2 }, doc)?.element).toBe(outer);
    hit = other;
    expect(targets.at({ x: 1, y: 2 }, doc)).toBeNull();
    hit = null;
    expect(targets.at({ x: 1, y: 2 }, doc)).toBeNull();
    offOuter();
  });

  it('offers a drop to the target under it first; what it refuses goes on to the features and the logs', async () => {
    const targets = new DropTargets();
    const el = document.createElement('form');
    document.body.appendChild(el);
    const images: DroppedFilesHandler = async (paths) => paths.filter((p) => p.endsWith('shot.png'));
    targets.register({ kind: 'aiImages', element: el, accept: images });
    // jsdom has no layout: the element under the point is the form.
    document.elementFromPoint = (() => el) as typeof document.elementFromPoint;
    const captures: DroppedFilesHandler = async (paths) => paths.filter((p) => p.endsWith('.nettrace'));
    expect(await routeDrop(targets, [captures], ['/shot.png', '/fake.png', '/a.nettrace', '/b.log'], { x: 5, y: 5 })).toEqual(['/fake.png', '/b.log']);
    // No point (a build without positions) or no targets: the features alone decide.
    expect(await routeDrop(targets, [captures], ['/shot.png'], null)).toEqual(['/shot.png']);
    expect(await routeDrop(null, [], ['/shot.png'], { x: 0, y: 0 })).toEqual(['/shot.png']);
    el.remove();
  });
});

