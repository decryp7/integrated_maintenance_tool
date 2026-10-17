import { describe, expect, it } from 'vitest';
import { DroppedFilesHandler, routeDroppedFiles } from './dropped-files';

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
