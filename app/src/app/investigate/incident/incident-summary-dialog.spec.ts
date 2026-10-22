import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { AiStatusStore } from '../../ai/ai-status.store';
import { Report } from '../../core/api.types';
import { DIALOG_REF, DialogService } from '../../ui/dialog.service';
import { IncidentBackend } from '../investigate-backend';
import { IncidentSummaryDialog, openIncidentSummary } from './incident-summary-dialog';

const report = (): Report => ({ version: 1, id: 'i1', title: 'T', summary: '', createdAt: 1, updatedAt: 1, sections: [] });
const narr = { text: 'n', citations: [], evidence: { recordIds: [], queries: [] }, usage: { inputTokens: 0, outputTokens: 0 } };

function setup(hasModel = true) {
  const backend = { build: vi.fn(async () => report()), narrative: vi.fn(async () => narr) };
  const ref = { close: vi.fn(), closed: Promise.resolve(undefined) };
  TestBed.configureTestingModule({
    providers: [
      { provide: IncidentBackend, useValue: backend },
      { provide: AiStatusStore, useValue: { hasModel: signal(hasModel) } },
      { provide: DIALOG_REF, useValue: ref },
    ],
  });
  const fixture = TestBed.createComponent(IncidentSummaryDialog);
  fixture.detectChanges();
  const el = fixture.nativeElement as HTMLElement;
  const button = (name: string) => [...el.querySelectorAll('button')].find((b) => b.textContent?.trim() === name) as HTMLButtonElement;
  return { backend, ref, fixture, el, button };
}

describe('IncidentSummaryDialog', () => {
  it('renders the fields and the engine note', () => {
    const { el } = setup();
    expect((el.querySelector('#inc-title') as HTMLInputElement).value).toBe('Incident summary');
    expect((el.querySelector('#inc-top') as HTMLInputElement).value).toBe('5');
    expect(el.textContent).toContain('The engine computes every number');
    expect(el.textContent).toContain('Covers all the open data');
    expect((el.querySelector('input[type=checkbox]') as HTMLInputElement).checked).toBe(true);
  });

  it('disables the narrative without a model and says why', () => {
    const { el } = setup(false);
    const box = el.querySelector('input[type=checkbox]') as HTMLInputElement;
    expect([box.disabled, box.checked]).toEqual([true, false]);
    expect(el.textContent).toContain('Needs an AI model');
  });

  it('builds with the edited options and closes to show the report', async () => {
    const { el, fixture, button, backend, ref } = setup();
    const title = el.querySelector('#inc-title') as HTMLInputElement;
    title.value = 'Outage';
    title.dispatchEvent(new Event('input'));
    const top = el.querySelector('#inc-top') as HTMLInputElement;
    top.value = '40';
    top.dispatchEvent(new Event('change'));
    expect(top.value).toBe('10');
    (el.querySelector('input[type=checkbox]') as HTMLInputElement).click();
    button('Build').click();
    await fixture.whenStable();
    expect(backend.build).toHaveBeenCalledWith({ title: 'Outage', from: null, to: null, top: 10 });
    expect(backend.narrative).not.toHaveBeenCalled();
    expect(ref.close).toHaveBeenCalledWith('show');
  });

  it('shows a build error with Retry', async () => {
    const { fixture, el, button, backend, ref } = setup();
    backend.build.mockRejectedValueOnce(new Error('no data open'));
    button('Build').click();
    await fixture.whenStable();
    fixture.detectChanges();
    expect(el.querySelector('[role=alert]')?.textContent).toContain('no data open');
    expect(ref.close).not.toHaveBeenCalled();
    button('Retry').click();
    await fixture.whenStable();
    expect(backend.build).toHaveBeenCalledTimes(2);
    expect(ref.close).toHaveBeenCalledWith('show');
  });

  it('keeps the dialog open when only the narrative failed, and retries it', async () => {
    const { fixture, el, button, backend, ref } = setup();
    backend.narrative.mockRejectedValueOnce(new Error('model crashed'));
    button('Build').click();
    await fixture.whenStable();
    fixture.detectChanges();
    expect(el.querySelector('[role=alert]')?.textContent).toContain('narrative could not be added: model crashed');
    expect(ref.close).not.toHaveBeenCalled();
    button('Show report').click();
    expect(ref.close).toHaveBeenCalledWith('show');
    ref.close.mockClear();
    button('Retry narrative').click();
    await fixture.whenStable();
    expect(ref.close).toHaveBeenCalledWith('show');
  });

  it('keeps open when the narrative retry fails again', async () => {
    const { fixture, el, button, backend, ref } = setup();
    backend.narrative.mockRejectedValue(new Error('still failing'));
    button('Build').click();
    await fixture.whenStable();
    fixture.detectChanges();
    button('Retry narrative').click();
    await fixture.whenStable();
    expect(ref.close).not.toHaveBeenCalled();
    expect(el.textContent).toContain('still failing');
  });

  it('Cancel closes without a result', () => {
    const { button, ref } = setup();
    button('Cancel').click();
    expect(ref.close).toHaveBeenCalledWith();
  });

  it('opens through the dialog service', async () => {
    TestBed.configureTestingModule({});
    const dialogs = TestBed.inject(DialogService);
    const p = openIncidentSummary(dialogs);
    const open = dialogs.stack().at(0);
    expect(open?.options.title).toBe('Incident summary');
    open?.ref.close('show');
    expect(await p).toBe('show');
  });
});
