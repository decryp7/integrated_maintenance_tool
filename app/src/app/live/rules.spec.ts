import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyDataset, typeInto } from '../../testing/helpers';
import { AnalysisBackend } from '../core/analysis-backend';
import { Completion, QlError } from '../core/api.types';
import { MockBackend } from '../core/mock/mock-backend';
import { ProfileModel } from '../core/profile.model';
import { WorkspaceStore } from '../core/workspace.store';
import { parseProfile } from '../profile/toml';
import { DIALOG_REF } from '../ui/dialog.service';
import { AlertRulesEditor } from './alert-rules-editor';
import { errorTarget, LIVE_DEFAULTS, newRule, readLive, writeLive } from './live-rules.model';
import { LiveRulesProfile } from './live-rules.profile';
import { QL_CHECK_MS, QlInput } from './ql-input';
import { at } from '../../testing/must';

/** A profile save re-imports as a job: waits for it to finish and for the view to follow. */
async function applied(fixture?: Parameters<typeof settle>[0]): Promise<void> {
  await vi.waitFor(() => expect(TestBed.inject(WorkspaceStore).busy()).toBeNull(), { timeout: 5000 });
  await settle(fixture);
}

class FakeAnalysis {
  error: QlError | null = null;
  completions: Completion[] = [
    { label: 'level', insert: 'level', kind: 'field' },
    { label: 'message', insert: 'message', kind: 'field' },
  ];
  checkQuery = vi.fn(async (_t: string) => this.error);
  completeQuery = vi.fn(async (_t: string, _c: number) => this.completions);
}

describe('live rules model', () => {
  it('reads the engine defaults when [live] is absent', () => {
    expect(readLive({})).toEqual(LIVE_DEFAULTS);
  });

  it('reads and writes [live], keeping unknown keys', () => {
    const model: ProfileModel = {
      live: {
        alerts: ['error-spike', 'Silence', 'bogus', 'error_spike'],
        silence_after: 30,
        cooldown: '1m',
        clock: 'WALL',
        extra: 'kept',
        rule: [{ name: 'pay', when: 'level = "Error"', threshold: 3, window: '2m', severity: 'error' }, { when: 'x' }, null],
      },
    };
    const draft = readLive(model);
    expect(draft.detectors).toEqual(['error_spike', 'silence']);
    expect(draft.silenceAfter).toBe('30');
    expect(draft.window).toBe('5m');
    expect(draft.clock).toBe('wall');
    expect(draft.rules).toEqual([
      { name: 'pay', when: 'level = "Error"', threshold: 3, window: '2m', severity: 'critical' },
      { name: 'rule 2', when: 'x', threshold: 1, window: '1m', severity: 'warning' },
      { name: 'rule 3', when: '', threshold: 1, window: '1m', severity: 'warning' },
    ]);
    writeLive(model, { ...draft, detectors: ['silence', 'crash'], rules: [{ ...at(draft.rules, 0), name: ' pay ', threshold: 2.6, severity: 'info' }, { ...at(draft.rules, 1), threshold: Number.NaN }] });
    expect(model['live']).toEqual({
      alerts: ['silence', 'crash'],
      silence_after: '30',
      cooldown: '1m',
      window: '5m',
      clock: 'wall',
      extra: 'kept',
      rule: [
        { name: 'pay', when: 'level = "Error"', threshold: 3, window: '2m', severity: 'info' },
        { name: 'rule 2', when: 'x', threshold: 1, window: '1m', severity: 'warning' },
      ],
    });
    writeLive(model, { ...draft, rules: [] });
    expect((model['live'] as Record<string, unknown>)['rule']).toBeUndefined();
    expect(readLive({ live: 'nonsense' } as never).detectors.length).toBe(6);
    expect(at(readLive({ live: { rule: [{ severity: 'INFO' }] } }).rules, 0).severity).toBe('info');
    expect(newRule(1).name).toBe('rule 2');
  });

  it('maps engine messages to the rule or field they are about', () => {
    const rules = [newRule(0), { ...newRule(1), name: 'payments' }];
    expect(errorTarget('live.rule "payments".window: "x" is not a duration', rules)).toEqual({ kind: 'rule', index: 1 });
    expect(errorTarget('live.rule "rule 1": unexpected end', rules)).toEqual({ kind: 'rule', index: 0 });
    expect(errorTarget('live.rule "rule 9": x', rules)).toEqual({ kind: 'general' });
    expect(errorTarget('live.rule "gone": x', rules)).toEqual({ kind: 'general' });
    expect(errorTarget('live.cooldown: "x" is not a duration', rules)).toEqual({ kind: 'field', field: 'cooldown' });
    expect(errorTarget('live.alerts: unknown detector', rules)).toEqual({ kind: 'field', field: 'alerts' });
    expect(errorTarget('TOML parse error', rules)).toEqual({ kind: 'general' });
  });
});

describe('LiveRulesProfile and the rules editor', () => {
  async function setup() {
    const backend = new MockBackend({ latency: false, dataset: tinyDataset });
    const analysis = new FakeAnalysis();
    const close = vi.fn();
    TestBed.configureTestingModule({
      providers: [provideTinyBackend(backend), { provide: AnalysisBackend, useValue: analysis }, { provide: DIALOG_REF, useValue: { close, closed: Promise.resolve() } }],
    });
    const store = TestBed.inject(WorkspaceStore);
    await settle();
    return { backend, analysis, close, store, profile: TestBed.inject(LiveRulesProfile) };
  }

  it('validates in the engine before saving, and reports the line of an error', async () => {
    const { backend, profile, store } = await setup();
    const draft = await profile.load();
    expect(draft).toEqual(LIVE_DEFAULTS);
    const bad = { ...draft, rules: [{ ...newRule(0), window: 'soon' }] };
    const save = vi.spyOn(backend, 'saveProfile');
    const result = await profile.save(bad);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.failure.message).toContain('live.rule "rule 1".window');
      expect(result.failure.line).toBeGreaterThan(0);
      expect(result.failure.excerpt).toContain('soon');
    }
    expect(save).not.toHaveBeenCalled();
    expect((await profile.validate({ ...draft, rules: [newRule(0)] })).ok).toBe(true);
    const version = store.profileVersion();
    expect((await profile.save({ ...draft, rules: [newRule(0)] })).ok).toBe(true);
    await applied();
    expect(store.profileVersion()).toBe(version + 1);
    expect(parseProfile((await backend.getProfile()).toml)['live']).toMatchObject({ rule: [{ name: 'rule 1', threshold: 5 }] });
    vi.spyOn(backend, 'getProfile').mockRejectedValueOnce('disk gone');
    expect(await profile.validate(draft)).toEqual({ ok: false, failure: { message: 'disk gone', line: null, excerpt: null } });
    vi.spyOn(backend, 'validateProfile').mockResolvedValueOnce({ ok: false, error: null, line: null });
    expect(await profile.validate(draft)).toEqual({ ok: false, failure: { message: 'The profile is not valid.', line: null, excerpt: null } });
  });

  it('edits detectors, timing and rules, shows engine errors next to the rule, and saves', async () => {
    const { close, backend } = await setup();
    const fixture = TestBed.createComponent(AlertRulesEditor);
    fixture.detectChanges();
    expect(text(fixture)).toContain('Loading the profile');
    await settle(fixture);
    expect(all<HTMLInputElement>(fixture, 'input[type=checkbox]').filter((c) => c.checked).length).toBe(6);
    at(all<HTMLInputElement>(fixture, 'input[type=checkbox]'), 0).click();
    await settle(fixture);
    expect(text(fixture)).toContain('No alert rules');
    buttonByText(fixture, '+ Add rule').click();
    await settle(fixture);
    const rule = el(fixture, 'fieldset.rule');
    const inputs = [...rule.querySelectorAll<HTMLInputElement>('input.input')];
    typeInto(at(inputs, 0), 'payments', 'change');
    typeInto(at(inputs, 1), '3', 'change');
    typeInto(at(inputs, 2), 'later', 'change');
    typeInto(rule.querySelector('select')!, 'critical', 'change');
    await settle(fixture);
    buttonByText(fixture, 'Validate').click();
    await vi.waitFor(async () => {
      await settle(fixture);
      expect(el(fixture, 'fieldset.rule').classList.contains('invalid')).toBe(true);
    });
    expect(el(fixture, 'fieldset.rule [role=alert]').textContent).toContain('live.rule "payments".window');
    // Fix it: validation passes, Save applies and closes.
    typeInto(at([...el(fixture, 'fieldset.rule').querySelectorAll<HTMLInputElement>('input.input')], 2), '2m', 'change');
    await settle(fixture);
    buttonByText(fixture, 'Validate').click();
    await vi.waitFor(async () => {
      await settle(fixture);
      expect(text(fixture)).toContain('The rules are valid');
    });
    typeInto(at(all<HTMLInputElement>(fixture, '.row input.input'), 1), 'never', 'change');
    await settle(fixture);
    buttonByText(fixture, 'Save').click();
    await vi.waitFor(async () => {
      await settle(fixture);
      expect(text(fixture)).toContain('live.cooldown');
    });
    expect(close).not.toHaveBeenCalled();
    typeInto(at(all<HTMLInputElement>(fixture, '.row input.input'), 1), '5m', 'change');
    typeInto(at(all<HTMLInputElement>(fixture, '.row input.input'), 0), '30s', 'change');
    typeInto(at(all<HTMLInputElement>(fixture, '.row input.input'), 2), '10m', 'change');
    typeInto(at(all<HTMLSelectElement>(fixture, '.row select'), 0), 'wall', 'change');
    await settle(fixture);
    buttonByText(fixture, 'Save').click();
    await vi.waitFor(() => expect(close).toHaveBeenCalledWith(true));
    const live = parseProfile((await backend.getProfile()).toml)['live'] as Record<string, unknown>;
    expect(live).toMatchObject({ silence_after: '30s', cooldown: '5m', window: '10m', clock: 'wall', rule: [{ name: 'payments', threshold: 3, window: '2m', severity: 'critical' }] });
    expect((live['alerts'] as string[]).includes('error_spike')).toBe(false);
  });

  it('shows detector, general and load errors, and removes rules', async () => {
    const { backend } = await setup();
    vi.spyOn(backend, 'getProfile').mockRejectedValueOnce(new Error('no profile'));
    const broken = TestBed.createComponent(AlertRulesEditor);
    await settle(broken);
    expect(text(broken)).toContain('The profile could not be loaded: no profile');

    const fixture = TestBed.createComponent(AlertRulesEditor);
    await settle(fixture);
    buttonByText(fixture, '+ Add rule').click();
    await settle(fixture);
    all<HTMLButtonElement>(fixture, 'button').find((b) => b.getAttribute('aria-label') === 'Remove rule rule 1')!.click();
    await settle(fixture);
    expect(text(fixture)).toContain('No alert rules');
    vi.spyOn(backend, 'validateProfile').mockResolvedValueOnce({ ok: false, error: 'live.alerts: unknown detector "x"', line: null });
    buttonByText(fixture, 'Validate').click();
    await vi.waitFor(async () => {
      await settle(fixture);
      expect(el(fixture, 'fieldset.group').classList.contains('invalid')).toBe(true);
    });
    vi.spyOn(backend, 'validateProfile').mockResolvedValueOnce({ ok: false, error: 'TOML: bad', line: 2 });
    buttonByText(fixture, 'Validate').click();
    await vi.waitFor(async () => {
      await settle(fixture);
      expect(text(fixture)).toContain('TOML: bad');
      expect(text(fixture)).toContain('line 2:');
    });
  });

  it('the QL input suggests completions from the engine and shows syntax errors', async () => {
    const { analysis } = await setup();
    const fixture = TestBed.createComponent(QlInput);
    const changes: string[] = [];
    fixture.componentInstance.valueChange.subscribe((v) => changes.push(v));
    fixture.componentRef.setInput('label', 'Rule condition');
    await settle(fixture);
    const input = el<HTMLInputElement>(fixture, 'input');
    expect(input.getAttribute('aria-label')).toBe('Rule condition');
    typeInto(input, 'le');
    await new Promise((r) => setTimeout(r, QL_CHECK_MS + 20));
    await settle(fixture);
    expect(changes).toEqual(['le']);
    expect(analysis.completeQuery).toHaveBeenCalledWith('le', 2);
    expect(all(fixture, '[role=option]').length).toBe(2);
    expect(input.getAttribute('aria-expanded')).toBe('true');
    const key = (k: string, extra: KeyboardEventInit = {}) => input.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, ...extra }));
    key('ArrowDown');
    await settle(fixture);
    expect(at(all(fixture, '[role=option]'), 1).getAttribute('aria-selected')).toBe('true');
    key('ArrowUp');
    key('ArrowUp');
    await settle(fixture);
    expect(at(all(fixture, '[role=option]'), 1).getAttribute('aria-selected')).toBe('true');
    key('Enter');
    await settle(fixture);
    expect(input.value).toBe('message');
    expect(changes.at(-1)).toBe('message');
    key('Escape');
    await settle(fixture);
    expect(all(fixture, '[role=option]').length).toBe(0);
    analysis.error = { message: 'expected an operator', offset: 7, len: 0 };
    key(' ', { ctrlKey: true });
    await vi.waitFor(async () => {
      await settle(fixture);
      expect(text(fixture)).toContain('expected an operator');
    });
    expect(input.getAttribute('aria-invalid')).toBe('true');
    at(all<HTMLElement>(fixture, '[role=option]'), 0).click();
    await settle(fixture);
    expect(changes.at(-1)).toBe('level');
    key('Tab');
    input.dispatchEvent(new Event('blur'));
    await settle(fixture);
    expect(all(fixture, '[role=option]').length).toBe(0);
    analysis.completeQuery.mockRejectedValueOnce('down');
    await fixture.componentInstance.check();
    await settle(fixture);
    expect(all(fixture, '[role=option]').length).toBe(0);
    typeInto(input, '');
    await fixture.componentInstance.check();
    expect(analysis.checkQuery).not.toHaveBeenCalledWith('');
  });
});
