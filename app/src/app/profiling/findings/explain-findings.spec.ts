import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { ProfileFinding } from '../../core/api.types';
import { FakeContext } from '../../analysis/test-setup';
import { setupProfiling } from '../testing/profiling-test-support';
import { explainPrompt, FindingsExplainer } from './explain-findings';

const finding = (i: number, detail = 'detail'): ProfileFinding => ({
  kind: 'topSelf',
  severity: 'warning',
  title: `Finding ${i}`,
  detail,
  valuePct: 10,
  evidence: { view: 'callTree', profileId: 1, baselineId: null, frame: null, path: [], metric: 'timeNs', range: null, threads: null, eventKind: null },
});

describe('explainPrompt', () => {
  it('summarises the engine findings compactly', () => {
    const p = explainPrompt('orders', [finding(1), finding(2)], () => 'call tree');
    expect(p).toContain('"orders"');
    expect(p).toContain('- [warning] Finding 1. detail (evidence: call tree)');
    const many = explainPrompt('x', Array.from({ length: 30 }, (_, i) => finding(i)), () => '');
    expect(many.split('\n').length).toBe(11);
    const long = explainPrompt('x', [finding(1, 'y'.repeat(10_000))], () => '');
    expect(long.length).toBe(4000);
    expect(long.endsWith('…')).toBe(true);
  });
});

describe('FindingsExplainer', () => {
  it('shows the chat and sends the question', async () => {
    const doubles = setupProfiling();
    const { ChatStore } = await import('../../ai/chat/chat.store');
    const send = vi.fn(() => Promise.resolve(true));
    TestBed.overrideProvider(ChatStore, { useValue: { send } });
    const ok = await TestBed.inject(FindingsExplainer).explain('orders', [finding(1)], () => 'e');
    expect(ok).toBe(true);
    expect((doubles.context as FakeContext).last('showView')).toEqual(['ai-chat']);
    expect(send).toHaveBeenCalledWith(expect.stringContaining('Finding 1'));
  });
});
