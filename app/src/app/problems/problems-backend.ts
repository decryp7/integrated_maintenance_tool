import { inject, Injectable } from '@angular/core';
import { invoke } from '../core/activity/ipc';
import { DiagnosticRow, FixEvaluation, IndexStatus, QuickFix } from '../core/api.types';
import { DebriefBackend } from '../core/backend';
import { reportBackendFailures } from '../core/failure-reporter';
import { Logger, withFailureLogging } from '../core/logger';
import { browserMocks } from '../core/mock-registry';
import { isTauri } from '../core/platform';

/**
 * The import diagnostics commands (ADR 0014 §5): what went wrong while reading the window's sources, counted per
 * source and kind, and how a proposed parser change would do on the stored events. The engine decides both; the UI
 * shows them. Injected as a class token: the Tauri commands in the app, the mock over the mock workspace in a plain
 * browser. Every method rejects with a `CmdError`.
 */
@Injectable({ providedIn: 'root', useFactory: createProblemsBackend })
export abstract class ProblemsBackend {
  /** One row per source and kind of problem, plus one per file no parser read (`import_problems`). */
  abstract problems(): Promise<DiagnosticRow[]>;
  /** "Fixes `fixed` of `checked`": the fix re-parses the stored events it applies to; nothing changes (`evaluate_fix`). */
  abstract evaluateFix(fix: QuickFix): Promise<FixEvaluation>;
  /** How far the full-text index is (`index_status`). */
  abstract indexStatus(): Promise<IndexStatus>;
}

/** Talks to `workspace_commands::{import_problems, evaluate_fix, index_status}`. */
export class TauriProblemsBackend extends ProblemsBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }
  problems() {
    return this.call<DiagnosticRow[]>('import_problems');
  }
  evaluateFix(fix: QuickFix) {
    return this.call<FixEvaluation>('evaluate_fix', { fix });
  }
  indexStatus() {
    return this.call<IndexStatus>('index_status');
  }
}

export function createProblemsBackend(): ProblemsBackend {
  const backend: ProblemsBackend = isTauri() ? new TauriProblemsBackend() : new (browserMocks().MockProblemsBackend)(inject(DebriefBackend));
  return withFailureLogging<ProblemsBackend>(backend, inject(Logger), { onFailure: reportBackendFailures() });
}
