import { Directive } from '@angular/core';
import { measuredWidth } from '../profiling/flame/measured-width';

/** The width of the element it sits on, so a section can adapt to its own column and not to the pane around it. */
@Directive({ selector: '[debriefMeasuredWidth]', exportAs: 'debriefMeasuredWidth' })
export class MeasuredWidthOf {
  readonly width = measuredWidth(0);
}
