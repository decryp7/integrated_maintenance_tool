import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';
import { SourceInfo, TimestampIssue } from '../core/api.types';
import { formatCount } from '../core/format';
import { Icon } from '../shared/icon';
import { DialogService } from '../ui/dialog.service';

/** Characters of the example line quoted in the tooltip. */
const EXAMPLE_CHARS = 60;

/** "Timestamps not recognised in 1,204 lines (e.g. “Sat Nov 08 …”)": the warning's tooltip and accessible name. */
export function timestampWarningText(issue: TimestampIssue): string {
  const count = formatCount(issue.unparsedLines);
  const first = issue.examples[0];
  if (first === undefined) return $localize`:@@shell.timestampWarning.text:Timestamps not recognised in ${count}:count: lines`;
  const example = first.length > EXAMPLE_CHARS ? `${first.slice(0, EXAMPLE_CHARS)}…` : first;
  return $localize`:@@shell.timestampWarning.textExample:Timestamps not recognised in ${count}:count: lines (e.g. “${example}:example:”)`;
}

/**
 * The Explorer row's warning for a source whose timestamps the engine could not read (`timestampIssue`): an
 * icon button whose tooltip says how many lines and quotes one; it opens the format wizard with the format the
 * engine proposed.
 */
@Component({
  selector: 'imt-timestamp-warning',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    @if (source().timestampIssue; as issue) {
      <button type="button" class="icon-btn warn" data-testid="timestamp-warning" [attr.aria-label]="label()" [title]="label()" (click)="open(issue)">
        <imt-icon name="alert" [size]="13" />
      </button>
    }
  `,
  styles: `
    :host { display: contents; }
    .warn { width: 20px; height: 20px; flex: none; color: var(--warn-text); }
  `,
})
export class TimestampWarning {
  private readonly dialogs = inject(DialogService);

  readonly source = input.required<SourceInfo>();

  protected readonly label = computed(() => {
    const issue = this.source().timestampIssue;
    if (!issue) return '';
    const fix = $localize`:@@shell.timestampWarning.fix:Click to fix the timestamp format.`;
    return `${timestampWarningText(issue)}. ${fix}`;
  });

  protected async open(issue: TimestampIssue): Promise<void> {
    const { FormatWizard } = await import('../format/format-wizard');
    const s = this.source();
    this.dialogs.open(FormatWizard, {
      title: $localize`:@@shell.timestampWarning.dialog:Fix timestamps`,
      inputs: { path: s.path, name: s.name, timestampFormat: issue.suggestion?.format ?? null },
      width: 760,
    });
  }
}
