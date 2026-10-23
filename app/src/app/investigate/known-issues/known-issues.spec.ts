import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideFakeDock, provideTinyBackend, settle, text } from '../../../testing/helpers';
import { AiError } from '../../ai/ai-backend';
import { AiStatusStore } from '../../ai/ai-status.store';
import { AiStatus } from '../../ai/ai.types';
import { IssueMatch, IssueReport } from '../../core/api.types';
import { CmdError } from '../../core/cmd-error';
import { WorkspaceStore } from '../../core/workspace.store';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { CommandContext, DEBRIEF_COMMANDS, DebriefCommand } from '../../extensions/tokens';
import { PROBLEMS_VIEW } from '../../problems/problems-model';
import { ProblemsPanel } from '../../problems/problems-panel';
import { KnownIssuesBackend } from '../investigate-backend';
import { MockKnownIssuesBackend } from '../mock/mock-investigate-backends';
import { KnownIssues } from './known-issues';
import { KnownIssuesStore } from './known-issues.store';
import { knownIssuesCommands, provideKnownIssues } from './provide-known-issues';
import { DebriefBackend } from '../../core/backend';

const match = (patch: Partial<IssueMatch> = {}): IssueMatch => ({
  issue: { id: 'gw', title: 'Payment gateway timeouts', fix: 'Raise the timeout.\nCheck the provider.', matcher: { type: 'pattern', pattern: 'timed out' }, file: 'Payments' },
  origin: 'rule',
  query: '@message ~ "timed out"',
  count: 1234,
  sampleIds: [1, 2],
  ...patch,
});
const aiMatch = (): IssueMatch =>
  match({ origin: 'ai', query: 'message contains "refused"', issue: { ...match().issue, id: 'refused', title: 'Connection refused', file: 'Operations' } });
const report = (patch: Partial<IssueReport> = {}): IssueReport => ({ matches: [match()], checked: 2, truncated: 0, warnings: [], ...patch });

const status = (state: AiStatus['state'], available = true) => ({ available, state }) as AiStatus;

function setup(backend: Partial<KnownIssuesBackend> = {}) {
  const fake = { match: vi.fn(async () => report()), suggest: vi.fn(async () => [aiMatch()]), ...backend } as unknown as KnownIssuesBackend;
  const context = { showView: vi.fn(), setQuery: vi.fn(), jumpTo: vi.fn(), notifyError: vi.fn() } as unknown as CommandContext;
  TestBed.configureTestingModule({
    providers: [...provideTinyBackend(), ...provideFakeDock(), { provide: KnownIssuesBackend, useValue: fake }, { provide: COMMAND_CONTEXT, useValue: context }],
  });
  return { fake, context };
}

describe('KnownIssuesStore', () => {
  it('loads the report and clears the loading flag', async () => {
    const { fake } = setup();
    const store = TestBed.inject(KnownIssuesStore);
    expect(store.report()).toBeNull();
    const done = store.load();
    expect(store.loading()).toBe(true);
    await done;
    expect(store.loading()).toBe(false);
    expect(store.report()).toEqual(report());
    expect(fake.match).toHaveBeenCalled();
  });

  it('keeps the error text when matching fails, and recovers on the next load', async () => {
    const { fake } = setup();
    (fake.match as ReturnType<typeof vi.fn>).mockRejectedValueOnce(new CmdError({ code: 'busy', message: 'engine busy', retryable: true }));
    const store = TestBed.inject(KnownIssuesStore);
    await store.load();
    expect(store.error()).toBe('engine busy');
    await store.load();
    expect(store.error()).toBeNull();
    expect(store.report()?.matches).toHaveLength(1);
  });

  it('ignores a stale answer when a newer load started', async () => {
    let first!: (r: IssueReport) => void;
    const { fake } = setup();
    (fake.match as ReturnType<typeof vi.fn>).mockImplementationOnce(() => new Promise<IssueReport>((r) => (first = r)));
    const store = TestBed.inject(KnownIssuesStore);
    const a = store.load();
    const b = store.load();
    await b;
    first(report({ matches: [] }));
    await a;
    expect(store.report()?.matches).toHaveLength(1);
    expect(store.loading()).toBe(false);
  });

  it('suggests, with a flag while it runs, and clears the suggestions', async () => {
    setup();
    const store = TestBed.inject(KnownIssuesStore);
    const done = store.suggest();
    expect(store.suggesting()).toBe(true);
    void store.suggest(); // a second click while running does nothing
    await done;
    expect(store.suggesting()).toBe(false);
    expect(store.suggestions()).toEqual([aiMatch()]);
    store.clearSuggestions();
    expect(store.suggestions()).toEqual([]);
  });

  it.each(['noModel', 'unavailable'] as const)('a %s error is a friendly message, not a crash', async (code) => {
    setup({ suggest: vi.fn(async () => Promise.reject(new AiError(code, 'raw'))) });
    const store = TestBed.inject(KnownIssuesStore);
    await store.suggest();
    expect(store.suggestError()).toContain('Settings > AI');
    expect(store.suggestions()).toEqual([]);
    store.clearSuggestions();
    expect(store.suggestError()).toBeNull();
  });

  it('shows another suggest failure as its message', async () => {
    setup({ suggest: vi.fn(async () => Promise.reject(new AiError('engine', 'model crashed'))) });
    const store = TestBed.inject(KnownIssuesStore);
    await store.suggest();
    expect(store.suggestError()).toBe('model crashed');
  });

  it('matches again, coalesced, when the workspace changes while the section is visible', async () => {
    const { fake } = setup();
    const store = TestBed.inject(KnownIssuesStore);
    await TestBed.inject(WorkspaceStore).init();
    TestBed.tick();
    expect(fake.match).not.toHaveBeenCalled(); // nobody shows the section
    const f = TestBed.createComponent(KnownIssues);
    await settle(f);
    expect(fake.match).toHaveBeenCalledTimes(1);
    expect(store.report()).not.toBeNull();
  });
});

describe('KnownIssues component', () => {
  const withModel = () => TestBed.inject(AiStatusStore).setStatus(status('ready'));

  it('lists each match with fix, count, file and a badge by origin, and shows its events', async () => {
    const { context } = setup();
    withModel();
    const f = TestBed.createComponent(KnownIssues);
    await settle(f);
    expect(text(f)).toContain('Known issues (1)');
    const card = el(f, '[data-testid=known-issue-gw]');
    expect(card.textContent).toContain('Payment gateway timeouts');
    expect(el(f, '[data-testid=known-issue-fix]').textContent).toContain('Raise the timeout.');
    expect(el(f, '[data-testid=known-issue-count]').textContent).toContain('1,234');
    expect(el(f, '[data-testid=known-issue-origin]').textContent).toBe('From rule');
    expect(card.textContent).toContain('Payments');
    buttonByText(f, 'Show events').click();
    expect(context.setQuery).toHaveBeenCalledWith({ ql: '@message ~ "timed out"' });
    expect(context.showView).toHaveBeenCalledWith('events');
  });

  it('suggests with the AI, showing progress, then the suggestions with the AI badge', async () => {
    let finish!: (m: IssueMatch[]) => void;
    const { context } = setup({ suggest: vi.fn(() => new Promise<IssueMatch[]>((r) => (finish = r))) });
    withModel();
    const f = TestBed.createComponent(KnownIssues);
    await settle(f);
    buttonByText(f, 'Suggest with AI').click();
    await settle(f);
    expect(el(f, '[data-testid=known-issues-progress]')).toBeTruthy();
    expect(buttonByText(f, 'Suggesting…').disabled).toBe(true);
    finish([aiMatch()]);
    await settle(f);
    expect(text(f)).toContain('Known issues (2)');
    const badge = el(f, '[data-testid=known-issue-refused] [data-testid=known-issue-origin]');
    expect(badge.textContent).toBe('AI suggested, verified by the engine');
    el<HTMLButtonElement>(f, '[data-testid=known-issue-refused] [data-testid=known-issue-show]').click();
    expect(context.setQuery).toHaveBeenCalledWith({ ql: 'message contains "refused"' });
  });

  it('lets the user accept an AI suggestion for the report, and take it back', async () => {
    setup();
    withModel();
    const f = TestBed.createComponent(KnownIssues);
    await settle(f);
    expect(f.nativeElement.querySelector('[data-testid=known-issue-gw] [data-testid=known-issue-accept]')).toBeNull(); // rules need no acceptance
    await TestBed.inject(KnownIssuesStore).suggest();
    f.detectChanges();
    const accept = el<HTMLButtonElement>(f, '[data-testid=known-issue-refused] [data-testid=known-issue-accept]');
    expect(accept.textContent).toContain('Accept for report');
    accept.click();
    f.detectChanges();
    expect(accept.getAttribute('aria-pressed')).toBe('true');
    expect(accept.textContent).toContain('Accepted');
    expect(TestBed.inject(KnownIssuesStore).acceptedSuggestions().map((m) => m.issue.id)).toEqual(['refused']);
    accept.click();
    f.detectChanges();
    expect(TestBed.inject(KnownIssuesStore).acceptedSuggestions()).toEqual([]);
    accept.click();
    TestBed.inject(KnownIssuesStore).clearSuggestions();
    expect(TestBed.inject(KnownIssuesStore).accepted()).toEqual([]);
  });

  it('selects the evidence: filters to the match query and selects its first sample event', async () => {
    const { context } = setup();
    const f = TestBed.createComponent(KnownIssues);
    await settle(f);
    el<HTMLButtonElement>(f, '[data-testid=known-issue-gw] [data-testid=known-issue-select]').click();
    expect(context.setQuery).toHaveBeenCalledWith({ ql: '@message ~ "timed out"' });
    expect(context.showView).toHaveBeenCalledWith('events');
    expect(context.jumpTo).toHaveBeenCalledWith(1);
  });

  it('selects no event when the match has no samples', async () => {
    const { context } = setup({ match: vi.fn(async () => report({ matches: [match({ sampleIds: [] })] })) });
    const f = TestBed.createComponent(KnownIssues);
    await settle(f);
    el<HTMLButtonElement>(f, '[data-testid=known-issue-select]').click();
    expect(context.setQuery).toHaveBeenCalled();
    expect(context.jumpTo).not.toHaveBeenCalled();
  });

  it('collapses and expands the section; it starts open only when there is something to show', async () => {
    setup({ match: vi.fn(async () => report({ matches: [] })) });
    const f = TestBed.createComponent(KnownIssues);
    await settle(f);
    const toggle = el<HTMLButtonElement>(f, '[data-testid=known-issues-toggle]');
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(el(f, '#known-issues-body').hasAttribute('hidden')).toBe(true);
    toggle.click();
    f.detectChanges();
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(el(f, '#known-issues-body').hasAttribute('hidden')).toBe(false);
  });

  it('is open by default when issues match, and the user can collapse it', async () => {
    setup();
    const f = TestBed.createComponent(KnownIssues);
    await settle(f);
    const toggle = el<HTMLButtonElement>(f, '[data-testid=known-issues-toggle]');
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    toggle.click();
    f.detectChanges();
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
  });

  it('disables the AI button with a hint when there is no model', async () => {
    setup();
    const f = TestBed.createComponent(KnownIssues);
    await settle(f);
    TestBed.inject(AiStatusStore).setStatus(status('noModel')); // after the store's own refresh from the mock AI
    await settle(f);
    const b = buttonByText(f, 'Suggest with AI');
    expect(b.disabled).toBe(true);
    expect(b.title).toBe('Needs an AI model: set one up in Settings > AI');
    expect(el(f, '[data-testid=known-issues-no-model]')).toBeTruthy();
  });

  it('shows the friendly error when the engine says there is no model, and dismisses it', async () => {
    setup({ suggest: vi.fn(async () => Promise.reject(new AiError('noModel', 'raw'))) });
    withModel();
    const f = TestBed.createComponent(KnownIssues);
    await settle(f);
    buttonByText(f, 'Suggest with AI').click();
    await settle(f);
    expect(el(f, '[data-testid=known-issues-suggest-error]').textContent).toContain('Settings > AI');
    el<HTMLButtonElement>(f, '[data-testid=known-issues-suggest-error] button').click();
    await settle(f);
    expect(all(f, '[data-testid=known-issues-suggest-error]')).toHaveLength(0);
  });

  it('explains how to declare a known issue when nothing matches', async () => {
    setup({ match: vi.fn(async () => report({ matches: [] })) });
    withModel();
    const f = TestBed.createComponent(KnownIssues);
    await settle(f);
    const empty = el(f, '[data-testid=known-issues-empty]');
    expect(empty.textContent).toContain('Settings > AI > Knowledge');
    expect(el(f, '[data-testid=known-issues-empty] pre').textContent).toContain('```known-issue');
    expect(el(f, '[data-testid=known-issues-empty] pre').textContent).toContain('pattern:');
  });

  it('lists the notes that need fixing in a collapsible', async () => {
    setup({ match: vi.fn(async () => report({ warnings: ['Payments, known issue 2: it needs a `fix:`', 'second'] })) });
    withModel();
    const f = TestBed.createComponent(KnownIssues);
    await settle(f);
    expect(el(f, '[data-testid=known-issues-warnings] summary').textContent).toContain('2 knowledge notes need fixing');
    expect(all(f, '[data-testid=known-issues-warnings] li')).toHaveLength(2);
  });

  it('shows a load failure with Retry', async () => {
    const { fake } = setup();
    (fake.match as ReturnType<typeof vi.fn>).mockRejectedValueOnce('engine busy');
    withModel();
    const f = TestBed.createComponent(KnownIssues);
    await settle(f);
    expect(text(f)).toContain('could not be checked: engine busy');
    expect(all(f, '[data-testid=known-issues-empty]')).toHaveLength(0);
    buttonByText(f, 'Retry').click();
    await settle(f);
    expect(text(f)).toContain('Known issues (1)');
  });

  it('refreshes from its button', async () => {
    const { fake } = setup();
    withModel();
    const f = TestBed.createComponent(KnownIssues);
    await settle(f);
    el<HTMLButtonElement>(f, '[data-testid=known-issues-refresh]').click();
    await settle(f);
    expect(fake.match).toHaveBeenCalledTimes(2);
  });

  it('is part of the Problems pane', async () => {
    setup();
    withModel();
    const f = TestBed.createComponent(ProblemsPanel);
    await settle(f);
    expect(el(f, 'debrief-known-issues')).toBeTruthy();
  });
});

describe('known-issues command', () => {
  it('opens the Problems view and checks again', async () => {
    const { fake, context } = setup();
    const store = TestBed.inject(KnownIssuesStore);
    const [cmd] = knownIssuesCommands(store);
    expect(cmd?.id).toBe('knownIssues.check');
    expect(cmd?.title).toBe('Problems: Check known issues');
    cmd?.run(context);
    expect(context.showView).toHaveBeenCalledWith(PROBLEMS_VIEW);
    await vi.waitFor(() => expect(fake.match).toHaveBeenCalled());
  });

  it('is registered through provideKnownIssues', () => {
    setup();
    TestBed.resetTestingModule();
    TestBed.configureTestingModule({
      providers: [...provideTinyBackend(), ...provideFakeDock(), provideKnownIssues(), { provide: COMMAND_CONTEXT, useValue: {} }],
    });
    const commands = TestBed.inject(DEBRIEF_COMMANDS) as unknown as (DebriefCommand | DebriefCommand[])[];
    expect(commands.flat().some((c) => c.id === 'knownIssues.check')).toBe(true);
  });
});

describe('with the real mock backend', () => {
  it('finds the sample issue and reports a missing model as the friendly message', async () => {
    TestBed.configureTestingModule({ providers: [...provideTinyBackend(), ...provideFakeDock(), { provide: COMMAND_CONTEXT, useValue: {} }] });
    let hasModel = true;
    TestBed.overrideProvider(KnownIssuesBackend, { useFactory: () => new MockKnownIssuesBackend(TestBed.inject(DebriefBackend), () => hasModel) });
    const store = TestBed.inject(KnownIssuesStore);
    await TestBed.inject(WorkspaceStore).init();
    await store.load();
    expect(store.error()).toBeNull();
    hasModel = false;
    await store.suggest();
    expect(store.suggestError()).toContain('Settings > AI');
  });
});
