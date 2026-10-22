import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { SourceInfo, WorkspaceSummary } from '../core/api.types';
import { mockParserProposal } from '../core/mock/mock-ai-data';
import { WorkspaceStore } from '../core/workspace.store';
import { NotificationsStore } from '../notifications/notifications.store';
import { AiLauncher } from './ai-launcher';
import { ParserProposal } from './ai.types';
import { PARSER_SUGGEST_NOTICE, ParserSuggestNotice, parserSuggestion } from './parser-suggest-notice';
import { ParserVerdictView } from './proposals/parser-verdict';

function source(over: Partial<SourceInfo>): SourceInfo {
  return { id: 1, name: 'jobs.log', path: '/logs/jobs.log', skipped: 0, problems: [], parser: 'generic', ...over } as SourceInfo;
}

function summary(sources: SourceInfo[]): WorkspaceSummary {
  return { sources, unrecognized: [] } as unknown as WorkspaceSummary;
}

describe('parser generation (ADR 0032)', () => {
  afterEach(() => TestBed.resetTestingModule());

  function render(p: ParserProposal): HTMLElement {
    const f = TestBed.createComponent(ParserVerdictView);
    f.componentRef.setInput('p', p);
    f.detectChanges();
    return f.nativeElement as HTMLElement;
  }

  it('shows the score, the lines not read, the zone, the checks and the rounds the engine reported', () => {
    const root = render(mockParserProposal());
    const text = root.textContent!.replace(/\s+/g, ' ');
    expect(root.querySelector('[data-testid=parser-score]')!.textContent).toBe('99.5%');
    expect(text).toContain('of 64 lines read with a timestamp and a level');
    expect(text).toContain('target reached');
    expect(text).toContain('1 lines not read');
    expect(text).toContain('41');
    expect(text).toContain('-- worker paused --');
    expect(text).toContain('no pattern');
    expect(root.querySelector('[data-testid=parser-zone]')!.textContent).toContain("+02:00 from the file's modification time");
    expect(text).toContain('Worker kept');
    expect(text).toContain('RequestId dropped');
    expect(text).toContain('QueueDepth kept');
    expect(text).toContain('Charts for the profile');
    expect(text).toContain('Round 1: 62%');
    expect(text).toContain('Round 2: 99.5%');
    expect(root.querySelector('.dropped')!.textContent).toContain('RequestId');
  });

  it('marks a best effort, a rejected timestamp rule, an undecided zone and the built-in case', () => {
    const base = mockParserProposal();
    const v = base.verdict!;
    const best = render({
      ...base,
      status: 'bestEffort',
      chartsToml: '',
      verdict: {
        ...v,
        score: 0.5,
        readsLevels: false,
        identity: [],
        metrics: [],
        timestamps: { ...v.timestamps, accepted: false, reason: 'only 2 of 4 events have a timestamp' },
        zone: { offset: 'UTC', source: 'asked', confidence: 0, detail: 'nothing in the file decides the zone' },
      },
    });
    const t = best.textContent!.replace(/\s+/g, ' ');
    expect(t).toContain('best after 2 rounds — edit to finish');
    expect(t).toContain('Timestamps: only 2 of 4 events have a timestamp');
    expect(t).toContain('not decided: please check');
    expect(t).not.toContain('and a level');
    expect(best.querySelector('details')!.hasAttribute('open')).toBe(true);
    expect(render({ ...base, status: 'builtin', rounds: [] }).textContent).toContain("read by IMT's parsers");
    expect(render({ ...base, verdict: null }).querySelector('[data-testid=parser-verdict]')).toBeNull();
  });

  it('suggests a parser for a file only the generic parser read and that has problems', () => {
    expect(parserSuggestion(null)).toBeNull();
    expect(parserSuggestion(summary([source({ parser: 'nlog', skipped: 9 })]))).toBeNull();
    expect(parserSuggestion(summary([source({})]))).toBeNull();
    const bad = source({ id: 7, problems: [{ kind: 'unmatchedLine', count: 3, sampleLines: [4] }] as SourceInfo['problems'] });
    expect(parserSuggestion(summary([source({}), bad]))!.id).toBe(7);
    expect(parserSuggestion(summary([source({ skipped: 2 })]))!.id).toBe(1);
  });

  it('offers it in the notification centre after an import, and starts the generation from there', () => {
    const importVersion = signal(0);
    const current = signal<WorkspaceSummary | null>(null);
    const postLocal = vi.fn();
    const propose = vi.fn().mockResolvedValue(undefined);
    TestBed.configureTestingModule({
      providers: [
        { provide: WorkspaceStore, useValue: { importVersion, summary: current } },
        { provide: NotificationsStore, useValue: { postLocal } },
        { provide: AiLauncher, useValue: { propose } },
      ],
    });
    TestBed.inject(ParserSuggestNotice);
    TestBed.tick();
    expect(postLocal).not.toHaveBeenCalled();
    current.set(summary([source({ id: 3, name: 'odd.log', skipped: 5 })]));
    importVersion.set(1);
    TestBed.tick();
    expect(postLocal).toHaveBeenCalledTimes(1);
    const notice = postLocal.mock.calls[0]![0];
    expect(notice.key).toBe(PARSER_SUGGEST_NOTICE);
    expect(notice.title).toBe('No parser reads odd.log well');
    expect(notice.actionLabel).toBe('Generate a parser with AI…');
    notice.run();
    expect(propose).toHaveBeenCalledWith('parser', { sourceId: 3, fileName: 'odd.log' });
    // Nothing to suggest: nothing posted.
    current.set(summary([source({ parser: 'nlog' })]));
    importVersion.set(2);
    TestBed.tick();
    expect(postLocal).toHaveBeenCalledTimes(1);
  });
});
