import { TestBed } from '@angular/core/testing';
import { provideTinyBackend } from '../../testing/helpers';
import { AiError } from '../ai/ai-backend';
import { AiStatusStore } from '../ai/ai-status.store';
import { Playbook, PlaybookStep, PlaybookStepResult, Report } from '../core/api.types';
import { DebriefBackend } from '../core/backend';
import { IncidentBackend, KnownIssuesBackend, PlaybookBackend, TauriIncidentBackend, TauriKnownIssuesBackend, TauriPlaybookBackend } from './investigate-backend';
import { MockIncidentBackend, MockKnownIssuesBackend } from './mock/mock-investigate-backends';

describe('the Tauri investigation backends', () => {
  it('map every operation to its command and arguments', async () => {
    const invoke = vi.fn(async (..._args: unknown[]) => null);
    const call = invoke as never;
    const issues = new TauriKnownIssuesBackend(call);
    await issues.match();
    await issues.suggest();
    expect(invoke.mock.calls).toEqual([['known_issues_match'], ['known_issues_suggest']]);

    invoke.mockClear();
    const incident = new TauriIncidentBackend(call);
    const report = { id: 'r' } as Report;
    await incident.build({ top: 3 } as never);
    await incident.narrative(report);
    expect(invoke.mock.calls).toEqual([
      ['incident_report_build', { request: { top: 3 } }],
      ['incident_narrative', { report }],
    ]);

    invoke.mockClear();
    const playbooks = new TauriPlaybookBackend(call);
    const playbook = { id: 'p' } as Playbook;
    const step = { id: 's' } as PlaybookStep;
    const result = { kind: 'done' } as unknown as PlaybookStepResult;
    await playbooks.library();
    await playbooks.save(playbook);
    await playbooks.remove('p');
    await playbooks.importFile('/a.json');
    await playbooks.exportFile(playbook, '/b.json');
    await playbooks.runStep(step);
    await playbooks.start(playbook, 'run1');
    await playbooks.record({ id: 'run1' } as never, 's', result, 'note');
    await playbooks.restore(undefined);
    expect(invoke.mock.calls.map((c) => c[0])).toEqual([
      'playbook_library', 'playbook_save', 'playbook_remove', 'playbook_import', 'playbook_export', 'playbook_run_step', 'playbook_start', 'playbook_record', 'playbook_restore',
    ]);
    expect(invoke.mock.calls.at(-1)).toEqual(['playbook_restore', { record: null }]);
  });
});

describe('the mock investigation backends', () => {
  it('follow the AI status: no model rejects with noModel, a model answers (one switch, ?mockAi=setup)', async () => {
    TestBed.configureTestingModule({ providers: provideTinyBackend() });
    const debrief = TestBed.inject(DebriefBackend);
    let hasModel = false;
    const issues = new MockKnownIssuesBackend(debrief, () => hasModel);
    const incident = new MockIncidentBackend(debrief, () => hasModel);
    await expect(issues.suggest()).rejects.toBeInstanceOf(AiError);
    await expect(issues.suggest()).rejects.toMatchObject({ code: 'noModel' });
    const built = await incident.build({ top: 5 } as never);
    await expect(incident.narrative(built)).rejects.toMatchObject({ code: 'noModel' });
    hasModel = true;
    await expect(incident.narrative(built)).resolves.toMatchObject({ text: expect.any(String) });
    await expect(issues.suggest()).resolves.toBeInstanceOf(Array);
  });

  it('are wired to the AI status store by the factories', async () => {
    TestBed.configureTestingModule({ providers: provideTinyBackend() });
    const ai = TestBed.inject(AiStatusStore);
    ai.setStatus({ available: true, state: 'noModel' } as never);
    await expect(TestBed.inject(KnownIssuesBackend).suggest()).rejects.toMatchObject({ code: 'noModel' });
    ai.setStatus({ available: true, state: 'ready' } as never);
    await expect(TestBed.inject(KnownIssuesBackend).suggest()).resolves.toBeDefined();
    expect(TestBed.inject(IncidentBackend)).toBeDefined();
    expect(TestBed.inject(PlaybookBackend)).toBeDefined();
  });
});
