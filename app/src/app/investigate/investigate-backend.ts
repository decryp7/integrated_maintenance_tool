import { inject, Injectable } from '@angular/core';
import { invoke } from '../core/activity/ipc';
import { IncidentRequest, IssueReport, IssueMatch, Playbook, PlaybookLibrary, PlaybookRestore, PlaybookRun, PlaybookStepResult, PlaybookStep, Report } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { reportBackendFailures } from '../core/failure-reporter';
import { Logger, withFailureLogging } from '../core/logger';
import { browserMocks } from '../core/mock-registry';
import { isTauri } from '../core/platform';
import type { Narrative } from '../ai/ai.types';

/**
 * Known issues (ADR 0039): the knowledge files' declared issues matched by the engine, and the AI's suggestions
 * (always verified by the engine; they need a model and reject with an AI error code when there is none).
 */
@Injectable({ providedIn: 'root', useFactory: createKnownIssuesBackend })
export abstract class KnownIssuesBackend {
  /** The declared issues present in the open data, with evidence (`known_issues_match`). */
  abstract match(): Promise<IssueReport>;
  /** Issues the model found in the knowledge notes, each verified by the engine (`known_issues_suggest`). */
  abstract suggest(): Promise<IssueMatch[]>;
}

/** The incident summary: the engine builds the report, the model may add a marked narrative. */
@Injectable({ providedIn: 'root', useFactory: createIncidentBackend })
export abstract class IncidentBackend {
  /** The summary of the open data as an ordinary report (`incident_report_build`). */
  abstract build(request: IncidentRequest): Promise<Report>;
  /** The AI narrative of a report, citations checked (`incident_narrative`). */
  abstract narrative(report: Report): Promise<Narrative>;
}

/** Investigation playbooks: the library, running a step, recording a run. */
@Injectable({ providedIn: 'root', useFactory: createPlaybookBackend })
export abstract class PlaybookBackend {
  abstract library(): Promise<PlaybookLibrary>;
  abstract save(playbook: Playbook): Promise<Playbook>;
  abstract remove(id: string): Promise<boolean>;
  abstract importFile(path: string): Promise<Playbook>;
  abstract exportFile(playbook: Playbook, path: string): Promise<void>;
  /** Runs one step against the open data (`playbook_run_step`). */
  abstract runStep(step: PlaybookStep): Promise<PlaybookStepResult>;
  abstract start(playbook: Playbook, runId: string): Promise<PlaybookRun>;
  abstract record(run: PlaybookRun, stepId: string, result: PlaybookStepResult, note: string): Promise<PlaybookRun>;
  /** Checks the runs a project saved (null clears them). */
  abstract restore(record: unknown): Promise<PlaybookRestore>;
}

export class TauriKnownIssuesBackend extends KnownIssuesBackend {
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }
  match() {
    return this.call<IssueReport>('known_issues_match');
  }
  suggest() {
    return this.call<IssueMatch[]>('known_issues_suggest');
  }
}

export class TauriIncidentBackend extends IncidentBackend {
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }
  build(request: IncidentRequest) {
    return this.call<Report>('incident_report_build', { request });
  }
  narrative(report: Report) {
    return this.call<Narrative>('incident_narrative', { report });
  }
}

export class TauriPlaybookBackend extends PlaybookBackend {
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }
  library() {
    return this.call<PlaybookLibrary>('playbook_library');
  }
  save(playbook: Playbook) {
    return this.call<Playbook>('playbook_save', { playbook });
  }
  remove(id: string) {
    return this.call<boolean>('playbook_remove', { id });
  }
  importFile(path: string) {
    return this.call<Playbook>('playbook_import', { path });
  }
  exportFile(playbook: Playbook, path: string) {
    return this.call<void>('playbook_export', { playbook, path });
  }
  runStep(step: PlaybookStep) {
    return this.call<PlaybookStepResult>('playbook_run_step', { step });
  }
  start(playbook: Playbook, runId: string) {
    return this.call<PlaybookRun>('playbook_start', { playbook, runId });
  }
  record(run: PlaybookRun, stepId: string, result: PlaybookStepResult, note: string) {
    return this.call<PlaybookRun>('playbook_record', { run, stepId, result, note });
  }
  restore(record: unknown) {
    return this.call<PlaybookRestore>('playbook_restore', { record: record ?? null });
  }
}

export function createKnownIssuesBackend(): KnownIssuesBackend {
  const backend: KnownIssuesBackend = isTauri() ? new TauriKnownIssuesBackend() : new (browserMocks().MockKnownIssuesBackend)(inject(ImtBackend));
  return withFailureLogging<KnownIssuesBackend>(backend, inject(Logger), { onFailure: reportBackendFailures() });
}

export function createIncidentBackend(): IncidentBackend {
  const backend: IncidentBackend = isTauri() ? new TauriIncidentBackend() : new (browserMocks().MockIncidentBackend)(inject(ImtBackend));
  return withFailureLogging<IncidentBackend>(backend, inject(Logger), { onFailure: reportBackendFailures() });
}

export function createPlaybookBackend(): PlaybookBackend {
  const backend: PlaybookBackend = isTauri() ? new TauriPlaybookBackend() : new (browserMocks().MockPlaybookBackend)(inject(ImtBackend));
  return withFailureLogging<PlaybookBackend>(backend, inject(Logger), { onFailure: reportBackendFailures() });
}
