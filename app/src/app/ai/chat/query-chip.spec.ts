import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { FakeContext, provideAnalysisTesting } from '../../analysis/test-setup';
import { MockAiBackend } from '../../core/mock/mock-ai-backend';
import { AiBackend } from '../ai-backend';
import { all, el, settle, typeInto } from '../../../testing/helpers';
import { AiChatPanel } from './chat-panel';
import { AiCitations } from './citation-actions';
import { ChatStore } from './chat.store';
import { reduceTurn, startTurn } from './chat-model';

const QL = 'level >= error and ProcessName = "CompanyA.Graphic.Builder.exe"';

function setup() {
  TestBed.configureTestingModule({
    providers: [...provideAnalysisTesting({ context: new FakeContext() }), { provide: AiBackend, useValue: new MockAiBackend({ stepMs: 0 }) }],
  });
  const f = TestBed.createComponent(AiChatPanel);
  return { f, store: TestBed.inject(ChatStore) };
}

async function until(f: { detectChanges(): void }, cond: () => boolean) {
  for (let i = 0; i < 300 && !cond(); i++) {
    await settle();
    f.detectChanges();
  }
  expect(cond(), String(cond)).toBe(true);
}

async function ask(f: ReturnType<typeof setup>['f'], text: string) {
  await until(f, () => !!f.nativeElement.querySelector('textarea') && !el<HTMLTextAreaElement>(f, 'textarea').disabled);
  typeInto(el<HTMLTextAreaElement>(f, 'textarea'), text);
  f.detectChanges();
  el<HTMLButtonElement>(f, 'button[type=submit]').click();
}

describe('the query chip of an assistant turn', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('keeps the queries and SQL of a finished tool on its chip', () => {
    let t = startTurn('q');
    t = reduceTurn(t, { type: 'toolStarted', name: 'run_query', args: '{}' });
    t = reduceTurn(t, { type: 'toolFinished', name: 'run_query', summary: '2 events', error: false, evidence: { recordIds: [1], queries: [QL] } });
    t = reduceTurn(t, { type: 'toolFinished', name: 'run_sql', summary: 'SQL: 1 rows', error: false, evidence: { recordIds: [], queries: [], sql: ['SELECT 1'] } });
    expect(t.tools.map((c) => [c.queries, c.sql])).toEqual([[[QL], []], [[], ['SELECT 1']]]);
  });

  it('shows the exact query for "show me errors from graphic builder" with Open in Events and Edit', async () => {
    const { f } = setup();
    f.detectChanges();
    const query = vi.spyOn(TestBed.inject(AiCitations), 'query').mockResolvedValue(null);
    await ask(f, 'show me errors from graphic builder');
    await until(f, () => !!f.nativeElement.querySelector('[data-testid=ai-query-chip]'));
    expect(el(f, '[data-testid=ai-query-text]').textContent).toBe(QL);
    expect(f.nativeElement.querySelector('[data-testid=ai-query-sql-tag]')).toBeNull();

    // Open in Events runs the text as shown.
    el<HTMLButtonElement>(f, '[data-testid=ai-query-open]').click();
    await settle();
    expect(query).toHaveBeenLastCalledWith(QL);

    // Edit in place, then open the edited text.
    el<HTMLButtonElement>(f, '[data-testid=ai-query-edit-start]').click();
    f.detectChanges();
    const input = el<HTMLInputElement>(f, '[data-testid=ai-query-edit]');
    expect(input.value).toBe(QL);
    typeInto(input, 'level >= warning and ProcessName = "CompanyA.Graphic.Builder.exe"');
    f.detectChanges();
    el<HTMLButtonElement>(f, '[data-testid=ai-query-open-edited]').click();
    await settle();
    f.detectChanges();
    expect(query).toHaveBeenLastCalledWith('level >= warning and ProcessName = "CompanyA.Graphic.Builder.exe"');
    expect(el(f, '[data-testid=ai-query-text]').textContent).toContain('level >= warning');
  });

  it('shows the SQL the assistant used, read-only and tagged', async () => {
    const { f } = setup();
    f.detectChanges();
    await ask(f, 'what is the 95th percentile of the gap between events?');
    await until(f, () => !!f.nativeElement.querySelector('[data-testid=ai-query-sql-tag]'));
    expect(el(f, '[data-testid=ai-query-text]').textContent).toContain('quantile_cont');
    expect(all(f, '[data-testid=ai-query-open]')).toHaveLength(0);
    expect(all(f, '[data-testid=ai-query-edit-start]')).toHaveLength(0);
  });
});
