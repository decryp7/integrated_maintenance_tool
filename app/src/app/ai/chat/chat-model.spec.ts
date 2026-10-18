import { describe, expect, it } from 'vitest';
import { QueryProposal, TurnRecord } from '../ai.types';
import { ALL_CONTEXT, buildContext, ContextSources, describeContext, reduceTurn, startTurn, toolLabel, turnFromRecord } from './chat-model';

const q: QueryProposal = { kind: 'query', ql: 'level >= error', explanation: 'e', estimatedMatches: 2, repaired: false };
const ev = { recordIds: [], queries: [] };

describe('reduceTurn', () => {
  it('folds a streamed turn', () => {
    let t = startTurn('why?');
    t = reduceTurn(t, { type: 'delta', text: 'Let me look' });
    t = reduceTurn(t, { type: 'toolStarted', name: 'run_query', args: '{}' });
    expect(t.text).toBe('');
    t = reduceTurn(t, { type: 'toolStarted', name: 'run_query', args: '{"b":1}' });
    t = reduceTurn(t, { type: 'toolFinished', name: 'run_query', summary: '3 events', error: false, evidence: ev });
    expect(t.tools.map((c) => [c.running, c.summary, c.args])).toEqual([
      [true, null, '{}'],
      [false, '3 events', '{"b":1}'],
    ]);
    // A finish without a start still shows a chip.
    t = reduceTurn(t, { type: 'toolFinished', name: 'get_events', summary: 'declined', error: true, evidence: ev });
    expect(t.tools[2]).toEqual({ name: 'get_events', args: '', running: false, summary: 'declined', error: true });
    t = reduceTurn(t, { type: 'proposal', proposal: q });
    t = reduceTurn(t, { type: 'delta', text: 'Done.' });
    const done = reduceTurn(t, { type: 'done', text: 'Final', citations: [{ kind: 'query', ql: 'a' }], usage: { inputTokens: 1, outputTokens: 2 }, notes: ['n'] });
    expect(done).toMatchObject({ text: 'Final', running: false, notes: ['n'], usage: { inputTokens: 1, outputTokens: 2 } });
    expect(done.tools.every((c) => !c.running)).toBe(true);
    expect(done.proposals).toEqual([{ proposal: q, state: 'open' }]);

    const failed = reduceTurn(t, { type: 'error', message: 'engine down', cancelled: false });
    expect([failed.error, failed.cancelled, failed.running]).toEqual(['engine down', false, false]);
    const stopped = reduceTurn(t, { type: 'error', message: 'cancelled', cancelled: true });
    expect([stopped.error, stopped.cancelled]).toEqual([null, true]);
  });
});

describe('turnFromRecord', () => {
  it('rebuilds a saved turn, with web sources as citations and bad proposals skipped', () => {
    const r: TurnRecord = {
      user: 'q',
      assistant: 'a',
      citations: [{ kind: 'web', title: 'A', url: 'https://a' }],
      proposals: [q, null as never, { foo: 1 } as never],
      web: [
        { title: 'A', url: 'https://a' },
        { title: 'B', url: 'https://b' },
      ],
      cancelled: true,
      usage: { inputTokens: 1, outputTokens: 1 },
    };
    const t = turnFromRecord(r);
    expect(t.citations.map((c) => (c.kind === 'web' ? c.url : ''))).toEqual(['https://a', 'https://b']);
    expect(t.proposals).toHaveLength(1);
    expect([t.running, t.cancelled, t.text]).toEqual([false, true, 'a']);
    const bare = turnFromRecord({ user: 'u', assistant: 'x', cancelled: false } as TurnRecord);
    expect([bare.citations, bare.proposals, bare.usage]).toEqual([[], [], null]);
  });
});

describe('helpers', () => {
  const src: ContextSources = { ql: ' level >= error ', range: { from: 1, to: 2 }, selectedIds: Array.from({ length: 25 }, (_, i) => i), instance: 'WEB01', view: 'grid' };

  it('builds the context from what is on and describes it', () => {
    expect(buildContext(src, ALL_CONTEXT)).toEqual({ query: 'level >= error', timeRange: [1, 2], selectedIds: src.selectedIds.slice(0, 20), focusedInstance: 'WEB01', openView: 'grid' });
    expect(buildContext(src, { filter: false, range: false, selection: false, instance: false, view: false })).toEqual({ query: null, timeRange: null, selectedIds: [], focusedInstance: null, openView: null });
    const lines = describeContext(src, (ms) => `t${ms}`);
    expect(lines.map((l) => l.label)).toEqual(['Filter: level >= error', 'Time: t1 – t2', 'Selected: 25 events (#0, #1, #2, …)', 'Instance: WEB01', 'View: grid']);
    const none = describeContext({ ql: null, range: null, selectedIds: [], instance: null, view: null }, String);
    expect(none.map((l) => l.available)).toEqual([false, false, false, false, false]);
    expect(none.map((l) => l.label)).toEqual(['Filter: none', 'Time: whole workspace', 'Selected: none', 'Instance: all', 'View: none']);
    expect(describeContext({ ...src, selectedIds: [5] }, String)[2]?.label).toBe('Selected: 1 event (#5)');
  });

  it('names tools for their chips', () => {
    expect(toolLabel('run_query')).toBe('Query');
    expect(toolLabel('get_events')).toBe('Read events');
    expect(toolLabel('my_tool')).toBe('my tool');
  });
});
