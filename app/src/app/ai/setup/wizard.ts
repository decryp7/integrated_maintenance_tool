/**
 * The set-up wizard as a pure state machine: loading the plan → choosing a model → running (download,
 * verify, validate, load, warm up) → ready | failed | cancelled. The engine decides everything (fit,
 * speed, stages); this only tracks where the user is.
 */
import { AiErrorCode, Recommendation, SetupEvent, SetupPlan } from '../ai.types';

export type RunningStage = Exclude<SetupEvent, { stage: 'ready' } | { stage: 'failed' } | { stage: 'cancelled' } | { stage: 'fellBackToCpu' }>;

export type WizardState =
  | { kind: 'loading' }
  | { kind: 'plan'; plan: SetupPlan; selectedId: string | null }
  | { kind: 'running'; modelId: string; stage: RunningStage; note: string | null; cancelling: boolean }
  | { kind: 'ready'; model: string; backend: string; note: string | null }
  | { kind: 'failed'; code: AiErrorCode; message: string; modelId: string | null }
  | { kind: 'cancelled' };

export type WizardEvent =
  | { type: 'planLoaded'; plan: SetupPlan; preferred?: string | null }
  | { type: 'failed'; code: AiErrorCode; message: string }
  | { type: 'select'; modelId: string }
  | { type: 'start' }
  | { type: 'setup'; event: SetupEvent }
  | { type: 'cancelRequested' }
  | { type: 'retry' };

export const INITIAL: WizardState = { kind: 'loading' };

/** Every catalog model, best first (the engine's ranking). */
export function choices(plan: SetupPlan): Recommendation[] {
  return plan.recommendations.ranked;
}

export function selected(state: WizardState): Recommendation | null {
  if (state.kind !== 'plan' || !state.selectedId) return null;
  return choices(state.plan).find((r) => r.model.id === state.selectedId) ?? null;
}

/** The chosen model can be set up (it is not too large for this machine). */
export function canStart(state: WizardState): boolean {
  const r = selected(state);
  return !!r && r.assessment.fit !== 'tooLarge';
}

export function next(state: WizardState, e: WizardEvent): WizardState {
  switch (e.type) {
    case 'planLoaded': {
      const ranked = choices(e.plan);
      const preferred = e.preferred && ranked.some((r) => r.model.id === e.preferred) ? e.preferred : null;
      return { kind: 'plan', plan: e.plan, selectedId: preferred ?? e.plan.chosen?.model.id ?? ranked[0]?.model.id ?? null };
    }
    case 'select':
      return state.kind === 'plan' && choices(state.plan).some((r) => r.model.id === e.modelId) ? { ...state, selectedId: e.modelId } : state;
    case 'start':
      return state.kind === 'plan' && canStart(state)
        ? { kind: 'running', modelId: state.selectedId!, stage: { stage: 'probing' }, note: null, cancelling: false }
        : state;
    case 'failed':
      return { kind: 'failed', code: e.code, message: e.message, modelId: state.kind === 'running' ? state.modelId : null };
    case 'cancelRequested':
      return state.kind === 'running' ? { ...state, cancelling: true } : state;
    case 'retry':
      return state.kind === 'failed' || state.kind === 'cancelled' ? INITIAL : state;
    case 'setup':
      return onSetup(state, e.event);
  }
}

function onSetup(state: WizardState, ev: SetupEvent): WizardState {
  // Terminal events are accepted from any state (the set-up may have been started elsewhere).
  switch (ev.stage) {
    case 'ready':
      return { kind: 'ready', model: ev.model, backend: ev.backend, note: state.kind === 'running' ? state.note : null };
    case 'failed':
      return { kind: 'failed', code: ev.code, message: ev.message, modelId: state.kind === 'running' ? state.modelId : null };
    case 'cancelled':
      return state.kind === 'running' || state.kind === 'plan' || state.kind === 'loading' ? { kind: 'cancelled' } : state;
    case 'fellBackToCpu':
      return state.kind === 'running' ? { ...state, note: ev.reason } : state;
    default:
      return state.kind === 'running' ? { ...state, stage: ev } : state;
  }
}

/** Plain words for the running stage. */
export function stageLabel(stage: RunningStage): string {
  switch (stage.stage) {
    case 'probing':
      return 'Checking this computer…';
    case 'recommending':
      return 'Choosing settings…';
    case 'downloading':
      return 'Downloading the model…';
    case 'verifying':
      return 'Verifying the download…';
    case 'validating':
      return 'Checking the model file…';
    case 'loading':
      return stage.gpu ? 'Loading the model on the GPU…' : 'Loading the model on the CPU…';
    case 'warmingUp':
      return 'Warming up…';
  }
}

/** Step number (1..5) of the running stage, for "Step n of 5". */
export function stageStep(stage: RunningStage): number {
  switch (stage.stage) {
    case 'probing':
    case 'recommending':
      return 1;
    case 'downloading':
      return 2;
    case 'verifying':
    case 'validating':
      return 3;
    case 'loading':
      return 4;
    case 'warmingUp':
      return 5;
  }
}
