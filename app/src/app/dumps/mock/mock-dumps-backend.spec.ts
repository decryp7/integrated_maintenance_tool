import { describe, expect, it } from 'vitest';
import { CmdError } from '../../core/cmd-error';
import { MOCK_DUMP_PATHS, MOCK_UMDH_PATHS, MockDumpsBackend } from './mock-dumps-backend';

const page = { offset: 0, limit: 100, filter: null };

describe('MockDumpsBackend', () => {
  it('sniffs, opens, lists and closes like the engine', async () => {
    const b = new MockDumpsBackend();
    expect((await b.sniff([MOCK_DUMP_PATHS[0]!, MOCK_UMDH_PATHS[0]!, '/x/readme.md'])).map((r) => r.kind)).toEqual(['dump', 'umdh', 'none']);
    const d = await b.open(MOCK_DUMP_PATHS[0]!);
    expect(d.kind).toBe('dump');
    // Reopening replaces the document.
    const again = await b.open(MOCK_DUMP_PATHS[0]!);
    expect(again.id).not.toBe(d.id);
    expect(await b.list()).toHaveLength(1);
    await expect(b.open('/x/readme.md')).rejects.toBeInstanceOf(CmdError);
    await expect(b.open('/missing/a.dmp')).rejects.toMatchObject({ code: 'notFound' });
    expect(await b.close(again.id)).toBe(true);
    expect(await b.close(again.id)).toBe(false);
    await expect(b.overview(again.id)).rejects.toMatchObject({ code: 'notFound' });
  });

  it('answers the dump queries', async () => {
    const b = new MockDumpsBackend();
    const { id } = await b.open(MOCK_DUMP_PATHS[0]!);
    expect((await b.overview(id)).exception?.name).toBe('CLR_EXCEPTION');
    const threads = await b.threads({ id, ...page });
    expect(threads.rows[0]!.state).toBe('faulted');
    expect((await b.threads({ id, ...page, filter: 'ledger' })).total).toBe(2);
    expect((await b.stack({ id, threadId: 4121 })).frames.some((f) => f.culprit)).toBe(true);
    await expect(b.stack({ id, threadId: 1 })).rejects.toMatchObject({ code: 'notFound' });
    expect((await b.modules({ id, ...page, filter: 'shop.' })).total).toBe(2);
    expect((await b.findings({ ids: [id] }))[0]!.kind).toBe('crashCause');
    expect(await b.findings({ ids: [] })).toEqual([]);
    expect(await b.explain({ ids: [id] })).toContain('OrderService.exe.4120.dmp');
    await expect(b.explain({ ids: [] })).rejects.toMatchObject({ code: 'invalid' });
    await expect(b.umdhDiff({ base: null, target: id, group: 'stack', ...page })).rejects.toMatchObject({ code: 'unsupported' });
    expect(await b.markers()).toHaveLength(1);
  });

  it('answers the UMDH queries', async () => {
    const b = new MockDumpsBackend();
    const ids: number[] = [];
    for (const p of MOCK_UMDH_PATHS) ids.push((await b.open(p)).id);
    await expect(b.overview(ids[0]!)).rejects.toMatchObject({ code: 'unsupported' });
    const [first, , last] = ids as [number, number, number];
    const stacks = await b.umdhDiff({ base: first, target: last, group: 'stack', ...page });
    expect(stacks.rows[0]!.key).toBe('cache');
    expect(stacks.rows[0]!.deltaBytes).toBe(20 << 20);
    expect((await b.umdhDiff({ base: first, target: last, group: 'module', ...page })).rows[0]!.key).toBe('Shop.Orders');
    expect((await b.umdhDiff({ base: null, target: last, group: 'function', ...page, filter: 'report' })).rows).toHaveLength(1);
    const s = await b.umdhStack({ base: first, target: last, key: 'cache' });
    expect(s.frames.find((f) => f.culprit)?.label).toContain('OrderCache::Add');
    await expect(b.umdhStack({ base: first, target: last, key: 'nope' })).rejects.toMatchObject({ code: 'notFound' });
    const roots = await b.umdhTree({ base: first, target: last, path: [], limit: 10 });
    expect(roots[0]!.key).toBe('OrderService!wmain');
    const kids = await b.umdhTree({ base: first, target: last, path: [roots[0]!.key], limit: 10 });
    expect(kids[0]!.deltaBytes).toBeGreaterThan(0);
    const series = await b.findings({ ids });
    expect(series.map((f) => f.kind)).toEqual(['leakGrowth', 'steadyGrowth', 'heapTotal']);
    expect((await b.findings({ ids: [first, last] })).map((f) => f.kind)).toEqual(['leakGrowth', 'heapTotal']);
    expect((await b.findings({ ids: [first] }))[0]!.kind).toBe('largeAllocation');
  });
});
