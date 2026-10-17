import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { QuerySession } from '../../analysis/query-session';
import { FakeContext, provideAnalysisTesting } from '../../analysis/test-setup';
import { MockAiBackend } from '../../core/mock/mock-ai-backend';
import { DialogService } from '../../ui/dialog.service';
import { all, buttonByText, el, settle, typeInto } from '../../../testing/helpers';
import { AiBackend, AiError } from '../ai-backend';
import { ApprovalDecision, Citation } from '../ai.types';
import { AiApprovalPrompt } from './approval-prompt';
import { ApprovalView } from './chat-model';
import { AiCitations } from './citation-actions';
import { AiRichText } from './rich-text';

function configure(ctx = new FakeContext(), backend = new MockAiBackend({ stepMs: 0 })) {
  TestBed.configureTestingModule({ providers: [...provideAnalysisTesting({ context: ctx }), { provide: AiBackend, useValue: backend }] });
  return { ctx, backend };
}

@Component({
  imports: [AiRichText],
  template: `<imt-ai-rich-text [text]="text()" [citations]="cites()" (record)="got.push(['record', $event])" (query)="got.push(['query', $event])" (link)="got.push(['link', $event])" />`,
})
class RichHost {
  readonly text = signal('');
  readonly cites = signal<Citation[]>([]);
  readonly got: unknown[] = [];
}

describe('AiRichText', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('renders model text as text: injected markup never becomes elements', () => {
    const f = TestBed.createComponent(RichHost);
    f.componentInstance.text.set('Hi <img src=x onerror=alert(1)> <script>alert(2)</script> [x](javascript:alert(3))\n\n```\n<a href="javascript:x">y</a>\n```');
    f.detectChanges();
    const root = f.nativeElement as HTMLElement;
    expect(root.querySelector('img, script, a, iframe')).toBeNull();
    expect(root.textContent).toContain('<img src=x onerror=alert(1)>');
    expect(root.textContent).toContain('<a href="javascript:x">y</a>');
  });

  it('renders every block and inline kind and emits evidence clicks', () => {
    const f = TestBed.createComponent(RichHost);
    f.componentInstance.cites.set([
      { kind: 'record', id: 12, verified: true },
      { kind: 'record', id: 13, verified: false },
      { kind: 'query', ql: 'level >= error' },
      { kind: 'web', title: 'Docs', url: 'https://d.example/x' },
    ]);
    f.componentInstance.text.set('## Head\nSee event 12 and event 13, `level >= error`, `code`, **b** *i* https://d.example/x\n\n- one\n\n1. two\n\n```\nfence\n```');
    f.detectChanges();
    const root = f.nativeElement as HTMLElement;
    expect(root.querySelector('[role=heading]')?.getAttribute('aria-level')).toBe('4');
    expect(root.querySelectorAll('ul li, ol li').length).toBe(2);
    expect(root.querySelector('pre code')?.textContent).toBe('fence');
    expect(root.querySelector('strong')?.textContent).toBe('b');
    expect(root.querySelector('em')?.textContent).toBe('i');
    const cites = all<HTMLButtonElement>(f, 'button.cite');
    expect(cites.map((b) => b.textContent)).toEqual(['12', '13', 'level >= error', 'https://d.example/x']);
    expect(cites[1].classList).toContain('unverified');
    cites.forEach((b) => b.click());
    expect(f.componentInstance.got).toEqual([
      ['record', 12],
      ['record', 13],
      ['query', 'level >= error'],
      ['link', { url: 'https://d.example/x', title: 'Docs' }],
    ]);
  });
});

@Component({
  imports: [AiApprovalPrompt],
  template: `<imt-ai-approval [approval]="a()" (decide)="got.push($event)" />`,
})
class ApprovalHost {
  readonly a = signal<ApprovalView>({ requestId: 1, kind: 'search', text: 'TimeoutException', domain: null, provider: 'brave', editable: true, state: 'pending' });
  readonly got: ApprovalDecision[] = [];
}

describe('AiApprovalPrompt', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('offers Send, Deny, Edit and allow-for-chat for a search', () => {
    const f = TestBed.createComponent(ApprovalHost);
    f.detectChanges();
    expect(el(f, 'code').textContent).toBe('TimeoutException');
    buttonByText(f, 'Send').click();
    buttonByText(f, 'Deny').click();
    buttonByText(f, 'Allow for this chat').click();
    buttonByText(f, 'Edit').click();
    f.detectChanges();
    const input = el<HTMLInputElement>(f, 'input');
    expect(input.value).toBe('TimeoutException');
    typeInto(input, '  ');
    f.detectChanges();
    expect(buttonByText(f, 'Send edited').disabled).toBe(true);
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    buttonByText(f, 'Back').click();
    f.detectChanges();
    buttonByText(f, 'Edit').click();
    f.detectChanges();
    typeInto(el<HTMLInputElement>(f, 'input'), 'timeout dotnet');
    f.detectChanges();
    el<HTMLInputElement>(f, 'input').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    f.detectChanges();
    expect(f.componentInstance.got).toEqual([{ kind: 'allow' }, { kind: 'deny' }, { kind: 'allowSearchesInChat' }, { kind: 'allowEdited', text: 'timeout dotnet' }]);
  });

  it('shows a fetch with its site and the outcome once decided', () => {
    const f = TestBed.createComponent(ApprovalHost);
    f.componentInstance.a.set({ requestId: 2, kind: 'fetch', text: 'https://x.example/p', domain: 'x.example', provider: 'brave', editable: false, state: 'pending' });
    f.detectChanges();
    const root = f.nativeElement as HTMLElement;
    expect(root.textContent).toContain('Read this web page:');
    expect(root.textContent).toContain('Site: x.example');
    expect(all(f, 'button').map((b) => b.textContent?.trim())).toEqual(['Deny', 'Send']);
    for (const [state, text] of [['allowedForChat', 'further searches'], ['denied', 'Not sent'], ['sending', 'Sending'], ['edited', 'edited']] as const) {
      f.componentInstance.a.update((a) => ({ ...a, state }));
      f.detectChanges();
      expect(el(f, '.result').textContent).toContain(text);
    }
  });
});

describe('AiCitations', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('jumps to events and applies queries', async () => {
    const { ctx } = configure();
    const c = TestBed.inject(AiCitations);
    await c.record(7);
    expect(ctx.last('jumpTo')).toEqual([7]);
    const session = TestBed.inject(QuerySession);
    const run = vi.spyOn(session, 'run').mockResolvedValueOnce('events').mockResolvedValueOnce('table').mockImplementationOnce(async () => {
        session.runError.set('bad syntax');
        return null;
      })
      .mockImplementationOnce(async () => {
        session.runError.set(null);
        return null;
      });
    expect(await c.query('level >= error')).toBeNull();
    expect(session.text()).toBe('level >= error');
    expect(ctx.last('showView')).toEqual(['query']);
    expect(await c.query('| stats count()')).toBeNull();
    expect(ctx.last('showView')).toEqual(['query-results']);
    expect(await c.query('x')).toBe('bad syntax');
    expect(await c.query('x')).toBe('The query could not be run.');
    expect(run).toHaveBeenCalledTimes(4);
  });

  it('opens a web page only after the user confirms', async () => {
    const { backend } = configure();
    const c = TestBed.inject(AiCitations);
    const dialogs = TestBed.inject(DialogService);
    let p = c.web('https://a.example', 'A');
    await settle();
    dialogs.stack()[0].ref.close();
    expect(await p).toBeNull();
    expect(backend.opened).toEqual([]);
    p = c.web('https://a.example', 'A');
    await settle();
    expect(dialogs.stack()[0].options.inputs).toMatchObject({ detail: 'https://a.example' });
    dialogs.stack()[0].ref.close(true);
    expect(await p).toBeNull();
    expect(backend.opened).toEqual(['https://a.example']);
    vi.spyOn(backend, 'openLink').mockRejectedValueOnce(new AiError('refused', 'refused: not part of an answer'));
    p = c.web('https://b.example', 'B');
    await settle();
    dialogs.stack()[0].ref.close(true);
    expect(await p).toBe('refused: not part of an answer');
  });
});
