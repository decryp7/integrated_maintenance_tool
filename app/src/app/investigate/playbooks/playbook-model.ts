import { Playbook, PlaybookAction, PlaybookExpect, PlaybookRun, PlaybookStatus, PlaybookStep, PlaybookStepRecord } from '../../core/api.types';

/** The id of the Playbooks pane. */
export const PLAYBOOKS_VIEW = 'playbooks';

/** What a step shows before it has a result. */
export type StepState = PlaybookStatus | 'pending';

export const ACTION_TYPES: PlaybookAction['type'][] = ['manual', 'query', 'filter', 'openView', 'check'];
export const EXPECT_KINDS: PlaybookExpect['kind'][] = ['noMatch', 'anyMatch', 'atLeast', 'atMost'];

export function statusLabel(s: StepState): string {
  switch (s) {
    case 'pass':
      return $localize`:@@playbooks.status.pass:Pass`;
    case 'fail':
      return $localize`:@@playbooks.status.fail:Fail`;
    case 'info':
      return $localize`:@@playbooks.status.info:Info`;
    case 'error':
      return $localize`:@@playbooks.status.error:Error`;
    case 'skipped':
      return $localize`:@@playbooks.status.skipped:Skipped`;
    case 'pending':
      return $localize`:@@playbooks.status.pending:Not run`;
  }
}

/** The icon beside a status (always shown with its text: colour is never the only cue). */
export function statusIcon(s: StepState): string {
  switch (s) {
    case 'pass':
      return 'check';
    case 'fail':
      return 'close';
    case 'info':
      return 'note';
    case 'error':
      return 'alert';
    case 'skipped':
      return 'minus';
    case 'pending':
      return 'more';
  }
}

export function actionLabel(t: PlaybookAction['type']): string {
  switch (t) {
    case 'manual':
      return $localize`:@@playbooks.action.manual:Manual step`;
    case 'query':
      return $localize`:@@playbooks.action.query:Query`;
    case 'filter':
      return $localize`:@@playbooks.action.filter:Filter`;
    case 'openView':
      return $localize`:@@playbooks.action.openView:Open a view`;
    case 'check':
      return $localize`:@@playbooks.action.check:Check`;
  }
}

export function expectLabel(k: PlaybookExpect['kind']): string {
  switch (k) {
    case 'noMatch':
      return $localize`:@@playbooks.expect.noMatch:No events match`;
    case 'anyMatch':
      return $localize`:@@playbooks.expect.anyMatch:Some events match`;
    case 'atLeast':
      return $localize`:@@playbooks.expect.atLeast:At least n events match`;
    case 'atMost':
      return $localize`:@@playbooks.expect.atMost:At most n events match`;
  }
}

/** The query text of an action, if it has one. */
export function actionQuery(a: PlaybookAction): string | null {
  return a.type === 'query' || a.type === 'filter' || a.type === 'check' ? a.ql : null;
}

/** An action of `type` with empty fields, for the editor. */
export function defaultAction(type: PlaybookAction['type']): PlaybookAction {
  switch (type) {
    case 'manual':
      return { type };
    case 'query':
    case 'filter':
      return { type, ql: '' };
    case 'openView':
      return { type, view: '' };
    case 'check':
      return { type, ql: '', expect: { kind: 'noMatch' } };
  }
}

export function defaultExpect(kind: PlaybookExpect['kind'], n = 1): PlaybookExpect {
  return kind === 'atLeast' || kind === 'atMost' ? { kind, n } : { kind };
}

/** Steps with a result, the run's own records by step id. */
export function recordOf(run: PlaybookRun, stepId: string): PlaybookStepRecord | undefined {
  return run.steps.find((r) => r.stepId === stepId);
}

export function stateOf(run: PlaybookRun, step: PlaybookStep): StepState {
  return recordOf(run, step.id)?.status ?? 'pending';
}

export interface RunProgress {
  done: number;
  total: number;
  passed: number;
  failed: number;
  errors: number;
  skipped: number;
  finished: boolean;
}

export function progressOf(run: PlaybookRun): RunProgress {
  const ids = new Set(run.playbook.steps.map((s) => s.id));
  const recs = run.steps.filter((r) => ids.has(r.stepId));
  const count = (s: PlaybookStatus) => recs.filter((r) => r.status === s).length;
  const total = run.playbook.steps.length;
  return { done: recs.length, total, passed: count('pass'), failed: count('fail'), errors: count('error'), skipped: count('skipped'), finished: total > 0 && recs.length >= total };
}

export function progressText(p: RunProgress): string {
  return $localize`:@@playbooks.progress:${p.done}:done: of ${p.total}:total: steps`;
}

export function finishedText(p: RunProgress): string {
  return $localize`:@@playbooks.finished:Finished: ${p.passed}:passed: passed, ${p.failed}:failed: failed, ${p.errors}:errors: errors, ${p.skipped}:skipped: skipped`;
}

/** A slug for a new playbook's id (lowercase letters, digits and dashes, as the engine requires). */
export function slugOf(title: string, taken: ReadonlySet<string>): string {
  const base = title.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 40) || 'playbook';
  let id = base;
  for (let n = 2; taken.has(id); n++) id = `${base}-${n}`;
  return id;
}

export function newPlaybook(taken: ReadonlySet<string>): Playbook {
  const title = $localize`:@@playbooks.new.title:New playbook`;
  return { version: 1, id: slugOf(title, taken), title, description: '', tags: [], steps: [newStep([])] };
}

export function newStep(existing: readonly PlaybookStep[]): PlaybookStep {
  const ids = new Set(existing.map((s) => s.id));
  let n = existing.length + 1;
  while (ids.has(`step-${n}`)) n++;
  return { id: `step-${n}`, title: $localize`:@@playbooks.new.step:New step`, instruction: '', action: { type: 'manual' } };
}

/** Moves item `i` by `delta` places (a copy; unchanged when it would leave the list). */
export function moved<T>(items: readonly T[], i: number, delta: number): T[] {
  const j = i + delta;
  const out = [...items];
  if (j < 0 || j >= out.length || i < 0 || i >= out.length) return out;
  const [item] = out.splice(i, 1);
  out.splice(j, 0, item as T);
  return out;
}

/** Early, convenient checks only: the engine validates on save. Returns the first thing to fix, or null. */
export function missingField(p: Playbook): string | null {
  if (!p.title.trim()) return $localize`:@@playbooks.missing.title:Give the playbook a title.`;
  if (p.steps.length === 0) return $localize`:@@playbooks.missing.steps:Add at least one step.`;
  for (const s of p.steps) {
    if (!s.title.trim()) return $localize`:@@playbooks.missing.stepTitle:Every step needs a title.`;
    const a = s.action;
    if ((a.type === 'query' || a.type === 'filter' || a.type === 'check') && !a.ql.trim()) return $localize`:@@playbooks.missing.ql:Step "${s.title}:title:" needs a query.`;
    if (a.type === 'openView' && !a.view.trim()) return $localize`:@@playbooks.missing.view:Step "${s.title}:title:" needs a view id.`;
  }
  return null;
}

/** Whether a step can be run by the engine (manual steps and views are done by hand). */
export function isRunnable(a: PlaybookAction): boolean {
  return a.type === 'query' || a.type === 'filter' || a.type === 'check';
}

export function newRunId(now: number, seq: number): string {
  return `run-${now.toString(36)}-${seq}`;
}
