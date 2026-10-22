import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { Narrative } from '../../ai/ai.types';
import { AiError } from '../../ai/ai-backend';
import { AiStatusStore } from '../../ai/ai-status.store';
import { TimeCursor } from '../../analysis/time-cursor';
import { Report } from '../../core/api.types';
import { ReportStore } from '../../share/report/report.store';
import { IncidentBackend } from '../investigate-backend';
import { AI_MARKER, clampTop, IncidentSummary, narrativeMarkdown } from './incident-summary';

const report = (): Report => ({
  version: 1,
  id: 'incident-1',
  title: 'Incident summary',
  summary: 's',
  createdAt: 1,
  updatedAt: 1,
  sections: [{ id: 's1', title: 'Summary', blocks: [{ id: 'b1', type: 'text', markdown: 'x' }] }],
});
const narrative = (verified = true): Narrative => ({
  text: ' The gateway timed out. ',
  citations: [
    { kind: 'record', id: 12, verified },
    { kind: 'record', id: 15, verified },
    { kind: 'record', id: 3, verified: true },
    { kind: 'query', ql: 'level = error' },
  ],
  evidence: { recordIds: [], queries: [] },
  usage: { inputTokens: 0, outputTokens: 0 },
});

function setup(hasModel = true) {
  const backend = { build: vi.fn(async () => report()), narrative: vi.fn(async () => narrative(false)) };
  TestBed.configureTestingModule({
    providers: [
      { provide: IncidentBackend, useValue: backend },
      { provide: AiStatusStore, useValue: { hasModel: signal(hasModel) } },
    ],
  });
  return { backend, svc: TestBed.inject(IncidentSummary), reports: TestBed.inject(ReportStore), cursor: TestBed.inject(TimeCursor) };
}

describe('IncidentSummary', () => {
  it('clamps the ranking size', () => {
    expect([clampTop(0), clampTop(99), clampTop('3'), clampTop('x'), clampTop(2.6)]).toEqual([1, 10, 3, 5, 3]);
    const { svc } = setup();
    svc.setTop(50);
    expect(svc.top()).toBe(10);
    expect(svc.request().top).toBe(10);
  });

  it('formats the narrative with the AI marker and lists unverified citations', () => {
    const md = narrativeMarkdown(narrative(false));
    expect(md.startsWith(`> ${AI_MARKER}\n\nThe gateway timed out.`)).toBe(true);
    expect(md.endsWith('Unverified citations: #12, #15')).toBe(true);
    expect(narrativeMarkdown(narrative(true))).not.toContain('Unverified');
  });

  it('builds with the options, the selected range and adopts the report with the narrative first', async () => {
    const { svc, backend, reports, cursor } = setup();
    cursor.select({ from: 10, to: 20 });
    svc.title.set('  Checkout outage ');
    expect(await svc.build()).toBe(true);
    expect(backend.build).toHaveBeenCalledWith({ title: 'Checkout outage', from: 10, to: 20, top: 5 });
    const active = reports.active();
    expect(active?.id).toBe('incident-1');
    expect(active?.sections.map((s) => s.title)).toEqual(['Narrative (AI-written)', 'Summary']);
    const block = active?.sections.at(0)?.blocks.at(0);
    expect(block?.type === 'text' && block.markdown).toContain(`> ${AI_MARKER}`);
    expect(block?.type === 'text' && block.markdown).toContain('Unverified citations: #12, #15');
    expect(svc.narrativeError()).toBeNull();
    expect(svc.building()).toBe(false);
  });

  it('covers all data without a selection and leaves the title default when blank', async () => {
    const { svc, backend } = setup();
    svc.title.set('  ');
    await svc.build();
    expect(backend.build).toHaveBeenCalledWith({ title: undefined, from: null, to: null, top: 5 });
  });

  it('skips the narrative when it is off or there is no model', async () => {
    const off = setup();
    off.svc.narrativeWanted.set(false);
    await off.svc.build();
    expect(off.backend.narrative).not.toHaveBeenCalled();
    TestBed.resetTestingModule();
    const none = setup(false);
    expect(await none.svc.build()).toBe(true);
    expect(none.backend.narrative).not.toHaveBeenCalled();
    expect(none.reports.active()?.sections).toHaveLength(1);
  });

  it('keeps the report when the narrative fails and lets the user retry', async () => {
    const { svc, backend, reports } = setup();
    backend.narrative.mockRejectedValueOnce(new AiError('noModel', 'No AI model is installed.'));
    expect(await svc.build()).toBe(true);
    expect(reports.active()?.sections).toHaveLength(1);
    expect(svc.narrativeError()).toContain('Settings > AI');
    expect(await svc.addNarrative()).toBe(true);
    expect(reports.active()?.sections.at(0)?.title).toBe('Narrative (AI-written)');
    expect(svc.narrativeError()).toBeNull();
    backend.narrative.mockRejectedValueOnce(new Error('engine busy'));
    svc.built.set(report());
    expect(await svc.addNarrative()).toBe(false);
    expect(svc.narrativeError()).toBe('engine busy');
  });

  it('reports a build failure without adopting anything, and has nothing to narrate then', async () => {
    const { svc, backend, reports } = setup();
    backend.build.mockRejectedValueOnce(new Error('no data open'));
    expect(await svc.build()).toBe(false);
    expect(svc.error()).toBe('no data open');
    expect(reports.reports()).toHaveLength(0);
    expect(await svc.addNarrative()).toBe(false);
  });

  it('ignores a second build while one runs', async () => {
    const { svc, backend } = setup();
    const first = svc.build();
    expect(await svc.build()).toBe(false);
    await first;
    expect(backend.build).toHaveBeenCalledTimes(1);
  });
});
