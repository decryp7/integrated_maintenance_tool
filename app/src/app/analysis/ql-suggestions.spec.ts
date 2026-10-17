import { TestBed } from '@angular/core/testing';
import { all, el, settle, text } from '../../testing/helpers';
import { QlError } from '../core/api.types';
import { AnalysisBackend } from '../core/analysis-backend';
import { QL_CHECK_MS, QlInput } from '../live/ql-input';
import { mockUnknownField } from './mock/mock-unknown-field';
import { applySuggestion } from './ql-text';
import { QlSuggestions } from './ql-suggestions';
import { CHECK_DELAY_MS, QuerySession } from './query-session';
import { QueryView } from './query-view';
import { loadWorkspace, provideAnalysisTesting, stubBackend } from './test-setup';

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
const unknown = (text: string, name: string, suggestions: string[]): QlError => {
  const offset = new TextEncoder().encode(text.slice(0, text.indexOf(name))).length;
  return { message: `unknown field "${name}"`, offset, len: new TextEncoder().encode(name).length, suggestions };
};

describe('did you mean (query language)', () => {
  it('replaces the reported byte span with the suggestion, quoting names that need it', () => {
    const text = 'msg = "é" and Levle = error';
    expect(applySuggestion(text, unknown(text, 'Levle', []), 'Level')).toEqual({ text: 'msg = "é" and Level = error', cursor: 19 });
    expect(applySuggestion('x = 1', unknown('x = 1', 'x', []), 'Machine Name').text).toBe('`Machine Name` = 1');
  });

  it('the mock flags names no event carries, with close suggestions (the engine does this for real)', () => {
    const fields = ['MachineName', 'ElapsedMs', 'machinename'];
    expect(mockUnknownField('Elapsedms > 5 and MachinName = "A"', fields)).toEqual({
      message: 'unknown field "MachinName"',
      offset: 18,
      len: 10,
      suggestions: ['MachineName'],
    });
    expect(mockUnknownField('levle >= error', fields)?.suggestions).toEqual(['level', '@level']);
    expect(mockUnknownField('zzzzzz contains "x"', fields)?.suggestions).toEqual([]);
    expect(mockUnknownField('message contains "é" and Q in x', fields)?.offset).toBe(26);
    expect(mockUnknownField('level >= error | stats count() by MachineName', fields)).toBeNull();
    expect(mockUnknownField('nope = 1', [])).toBeNull();
    expect(mockUnknownField('"unterminated', fields)).toBeNull();
  });

  it('renders suggestions as buttons and emits the choice', async () => {
    const fixture = TestBed.createComponent(QlSuggestions);
    await settle(fixture);
    expect(text(fixture)).toBe('');
    fixture.componentRef.setInput('error', unknown('Levle = 1', 'Levle', ['Level', 'LevelName']));
    await settle(fixture);
    const picked: string[] = [];
    fixture.componentInstance.pick.subscribe((n) => picked.push(n));
    expect(el(fixture, '[data-testid=did-you-mean]').textContent).toContain('Did you mean');
    const buttons = all<HTMLButtonElement>(fixture, 'button');
    expect(buttons.map((b) => b.getAttribute('aria-label'))).toEqual(['Replace with Level', 'Replace with LevelName']);
    buttons[1].click();
    expect(picked).toEqual(['LevelName']);
  });
});

describe('did you mean in the query bar and QL inputs', () => {
  it('fixes the query from the query bar', async () => {
    TestBed.configureTestingModule({ providers: provideAnalysisTesting() });
    await loadWorkspace();
    const fixture = TestBed.createComponent(QueryView);
    fixture.detectChanges();
    const input = el<HTMLInputElement>(fixture, 'input');
    input.focus();
    input.value = 'MachinName = "WEB01"';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await wait(CHECK_DELAY_MS + 80);
    await settle(fixture);
    expect(text(fixture)).toContain('unknown field "MachinName"');
    el<HTMLButtonElement>(fixture, '[data-testid=did-you-mean] button').click();
    await wait(CHECK_DELAY_MS + 80);
    await settle(fixture);
    expect(input.value).toBe('MachineName = "WEB01"');
    expect(TestBed.inject(QuerySession).error()).toBeNull();
    expect(input.selectionStart).toBe('MachineName'.length);
    expect(fixture.nativeElement.querySelector('[data-testid=did-you-mean]')).toBeNull();
  });

  it('fixes the condition of a QL input', async () => {
    const t = 'Levle = 1';
    const backend = stubBackend({ checkQuery: (async (q: string) => (q === t ? unknown(t, 'Levle', ['Level']) : null)) as never });
    TestBed.configureTestingModule({ providers: [...provideAnalysisTesting({ backend }), { provide: AnalysisBackend, useValue: backend }] });
    const fixture = TestBed.createComponent(QlInput);
    const changes: string[] = [];
    fixture.componentInstance.valueChange.subscribe((v) => changes.push(v));
    await settle(fixture);
    const input = el<HTMLInputElement>(fixture, 'input');
    input.value = t;
    input.dispatchEvent(new Event('input'));
    await wait(QL_CHECK_MS + 30);
    await settle(fixture);
    expect(text(fixture)).toContain('unknown field "Levle"');
    el<HTMLButtonElement>(fixture, '[data-testid=did-you-mean] button').click();
    await wait(QL_CHECK_MS + 30);
    await settle(fixture);
    expect(input.value).toBe('Level = 1');
    expect(changes.at(-1)).toBe('Level = 1');
    expect(text(fixture)).not.toContain('unknown field');
  });
});
