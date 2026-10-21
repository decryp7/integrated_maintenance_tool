import { describe, expect, it } from 'vitest';
import { TauriDumpsBackend } from './dumps-backend';

describe('TauriDumpsBackend', () => {
  it('calls one command per method with its arguments', async () => {
    const calls: [string, unknown][] = [];
    const invoke = (async (cmd: string, args?: unknown) => {
      calls.push([cmd, args]);
      return null;
    }) as never;
    const b = new TauriDumpsBackend(invoke);
    const req = { id: 1, offset: 0, limit: 10, filter: null };
    await b.sniff(['/a.dmp']);
    await b.open('/a.dmp');
    await b.list();
    await b.close(1);
    await b.overview(1);
    await b.threads(req);
    await b.stack({ id: 1, threadId: 2 });
    await b.modules(req);
    await b.findings({ ids: [1] });
    await b.umdhDiff({ base: null, target: 1, group: 'stack', offset: 0, limit: 5, filter: null });
    await b.umdhStack({ base: null, target: 1, key: 'k' });
    await b.umdhTree({ base: null, target: 1, path: [], limit: 5 });
    await b.markers();
    await b.explain({ ids: [1] });
    expect(calls.map((c) => c[0])).toEqual([
      'dumps_sniff',
      'dumps_open',
      'dumps_list',
      'dumps_close',
      'dumps_overview',
      'dumps_threads',
      'dumps_stack',
      'dumps_modules',
      'dumps_findings',
      'dumps_umdh_diff',
      'dumps_umdh_stack',
      'dumps_umdh_tree',
      'dumps_markers',
      'dumps_explain',
    ]);
    expect(calls[0]![1]).toEqual({ paths: ['/a.dmp'] });
    expect(calls[1]![1]).toEqual({ path: '/a.dmp' });
    expect(calls[2]![1]).toBeUndefined();
    expect(calls[3]![1]).toEqual({ id: 1 });
    expect(calls[5]![1]).toEqual({ request: req });
    expect(calls[13]![1]).toEqual({ request: { ids: [1] } });
  });
});
