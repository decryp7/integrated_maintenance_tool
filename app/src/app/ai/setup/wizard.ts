/**
 * The set-up dialog as a pure state machine: loading the plan → choosing a model, or failed. It is a modal choice
 * only: "Download and set up" hands the choice to the engine (a background job) and the dialog closes, so no
 * download, verification or load is tracked here. The engine decides everything (fit, speed); this only tracks
 * where the user is.
 */
import { AiErrorCode, Recommendation, Recommendations, SetupPlan } from '../ai.types';

export type WizardState =
  | { kind: 'loading' }
  | { kind: 'plan'; plan: SetupPlan; selectedId: string | null }
  | { kind: 'failed'; code: AiErrorCode; message: string };

export type WizardEvent =
  | { type: 'planLoaded'; plan: SetupPlan; preferred?: string | null }
  /** The engine read the model headers: the same models with exact memory figures (the selection stays). */
  | { type: 'fitRefined'; recommendations: Recommendations }
  | { type: 'failed'; code: AiErrorCode; message: string }
  | { type: 'select'; modelId: string }
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

function toPlan(plan: SetupPlan, preferredId: string | null): WizardState {
  const ranked = choices(plan);
  const preferred = preferredId && ranked.some((r) => r.model.id === preferredId) ? preferredId : null;
  return {
    kind: 'plan',
    plan,
    selectedId: preferred ?? plan.chosen?.model.id ?? ranked[0]?.model.id ?? null,
  };
}

export function next(state: WizardState, e: WizardEvent): WizardState {
  switch (e.type) {
    case 'planLoaded':
      return toPlan(e.plan, e.preferred ?? null);
    case 'fitRefined':
      return state.kind === 'plan' ? { ...state, plan: { ...state.plan, recommendations: e.recommendations } } : state;
    case 'select':
      return state.kind === 'plan' && choices(state.plan).some((r) => r.model.id === e.modelId) ? { ...state, selectedId: e.modelId } : state;
    case 'failed':
      return { kind: 'failed', code: e.code, message: e.message };
    case 'retry':
      return state.kind === 'failed' ? INITIAL : state;
  }
}
