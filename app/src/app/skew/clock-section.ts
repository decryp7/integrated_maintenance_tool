import { ChangeDetectionStrategy, Component, computed, inject, Injectable, input } from '@angular/core';
import { RecordView } from '../core/api.types';
import { DialogService } from '../ui/dialog.service';
import { ClockSkewStore } from './clock-skew.store';

/** Opens the "Adjust clock…" dialog of a source. */
@Injectable({ providedIn: 'root' })
export class ClockDialogService {
  private readonly dialogs = inject(DialogService);

  open(sourceId: number, sourceName = ''): void {
    this.dialogs.openLazy(() => import('./clock-dialog').then((m) => m.ClockDialog), { title: sourceName ? $localize`:@@skew.section.titleNamed:Adjust clock — ${sourceName}:name:` : $localize`:@@skew.section.title:Adjust clock`, inputs: { sourceId }, width: 560 });
  }
}

/** Inspector section: mark the event as the clock reference, or align its source on the reference. */
@Component({
  selector: 'debrief-clock-section',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="row">
      @if (isReference()) {
        <span class="faint small" data-testid="is-reference" i18n="@@skew.section.isReference">Clock reference for aligning other sources</span>
        <button type="button" class="btn ghost" (click)="skew.clearReference()" i18n="@@skew.section.clear">Clear</button>
      } @else {
        <button type="button" class="btn" (click)="skew.setReference(record())" i18n="@@skew.section.use">Use as clock reference</button>
      }
      @if (alignable()) {
        <button type="button" class="btn" (click)="adjust()" i18n="@@skew.section.align">Align {{ record().sourceName }} on reference…</button>
      }
    </div>
  `,
  styles: `
    .row { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; padding: 2px 0 6px; }
    .small { font-size: var(--fs-xs); }
  `,
})
export class ClockSection {
  readonly record = input.required<RecordView>();
  protected readonly skew = inject(ClockSkewStore);
  private readonly dialog = inject(ClockDialogService);

  protected readonly isReference = computed(() => this.skew.reference()?.id === this.record().id);
  protected readonly alignable = computed(() => {
    const ref = this.skew.reference();
    return !!ref && ref.source !== this.record().source;
  });

  protected adjust(): void {
    this.dialog.open(this.record().source, this.record().sourceName);
  }
}
