import { DatePipe } from '@angular/common';
import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { DIALOG_REF, DialogService } from '../../ui/dialog.service';
import { IncidentSummary, MAX_TOP } from './incident-summary';

/** What the dialog closes with: the report was built, show it. */
export type IncidentDialogResult = 'show';

/** Builds the incident summary report (the engine computes it); an AI narrative is optional and marked. */
@Component({
  selector: 'imt-incident-summary-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [DatePipe],
  template: `
    <div class="field">
      <label for="inc-title" i18n="@@incident.dialog.title">Title</label>
      <input id="inc-title" class="input" type="text" [value]="s.title()" (input)="s.title.set($any($event.target).value)" [disabled]="s.building()" />
    </div>
    <div class="field">
      <label for="inc-top" i18n="@@incident.dialog.top">Entries per ranking (1 to {{ max }})</label>
      <input id="inc-top" class="input num" type="number" min="1" [max]="max" [value]="s.top()" (change)="s.setTop($any($event.target).value); $any($event.target).value = s.top()" [disabled]="s.building()" />
    </div>
    <p class="faint" role="note">
      @if (s.period(); as p) {
        <span i18n="@@incident.dialog.periodSelected">Covers the selected time range: {{ p.from | date: 'medium' }} to {{ p.to | date: 'medium' }}.</span>
      } @else {
        <span i18n="@@incident.dialog.periodAll">Covers all the open data. Select a time range in the timeline first to limit it.</span>
      }
    </p>
    <label class="opt">
      <input type="checkbox" [checked]="s.narrativeWanted() && s.hasModel()" [disabled]="!s.hasModel() || s.building()" (change)="s.narrativeWanted.set($any($event.target).checked)" aria-describedby="inc-ai-note" />
      <span i18n="@@incident.dialog.narrative">Add an AI narrative</span>
    </label>
    @if (!s.hasModel()) {
      <p id="inc-ai-note" class="faint" i18n="@@incident.dialog.needsModel">Needs an AI model. Set one up in Settings > AI.</p>
    } @else {
      <p id="inc-ai-note" class="faint" i18n="@@incident.dialog.narrativeNote">The narrative is written by the local model from the report and marked as AI-written, with its cited events.</p>
    }
    <p class="faint" i18n="@@incident.dialog.note">The engine computes every number and event in the report; the AI only writes the optional narrative. Export it from the report view as HTML or PDF with redaction.</p>
    @if (s.building()) {
      <p role="status" class="progress" i18n="@@incident.dialog.building">Building the report…</p>
    }
    @if (s.error(); as e) {
      <div class="err" role="alert">
        <span>{{ e }}</span>
        <button class="btn" type="button" (click)="build()" i18n="@@incident.dialog.retry">Retry</button>
      </div>
    }
    @if (s.narrativeError(); as e) {
      <div class="err" role="alert">
        <span i18n="@@incident.dialog.narrativeFailed">The report was added, but the narrative could not be added: {{ e }}</span>
        <button class="btn" type="button" [disabled]="s.building()" (click)="retryNarrative()" i18n="@@incident.dialog.retryNarrative">Retry narrative</button>
      </div>
    }
    <div class="footer">
      @if (s.narrativeError()) {
        <button class="btn primary" type="button" (click)="ref.close('show')" i18n="@@incident.dialog.show">Show report</button>
      } @else {
        <button class="btn" type="button" (click)="ref.close()" i18n="@@incident.dialog.cancel">Cancel</button>
        <button class="btn primary" type="button" [disabled]="s.building()" (click)="build()" i18n="@@incident.dialog.build">Build</button>
      }
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 12px; }
    .field { display: flex; flex-direction: column; gap: 6px; }
    label { font-size: var(--fs-sm); color: var(--muted); }
    .opt { display: flex; align-items: center; gap: 6px; color: var(--text-2); font-size: var(--fs-md); min-height: 24px; }
    p { margin: 0; font-size: var(--fs-sm); }
    .faint { color: var(--muted); }
    .num { width: 6em; }
    .err { display: flex; align-items: center; justify-content: space-between; gap: 8px; color: var(--danger-text); }
    .footer { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class IncidentSummaryDialog {
  protected readonly ref = inject(DIALOG_REF);
  protected readonly s = inject(IncidentSummary);
  protected readonly max = MAX_TOP;

  protected async build(): Promise<void> {
    const ok = await this.s.build();
    if (ok && !this.s.narrativeError()) this.ref.close('show');
  }

  protected async retryNarrative(): Promise<void> {
    if (await this.s.addNarrative()) this.ref.close('show');
  }
}

/** Opens the dialog; resolves 'show' when a report was built and should be shown. */
export function openIncidentSummary(dialogs: DialogService): Promise<IncidentDialogResult | undefined> {
  return (dialogs.open<IncidentDialogResult>(IncidentSummaryDialog, { title: $localize`:@@incident.dialog.heading:Incident summary`, width: 520 }).closed);
}
