import { inject, Injectable } from '@angular/core';
import { WorkspaceStore } from '../../core/workspace.store';
import { DebriefCommand } from '../../extensions/tokens';
import { VisualExportService } from './visual-export.service';

/** Commands for chart and timeline images (charts export from their card's buttons). */
@Injectable({ providedIn: 'root' })
export class VisualCommands {
  private readonly visuals = inject(VisualExportService);
  private readonly store = inject(WorkspaceStore);

  readonly commands: DebriefCommand[] = [
    { id: 'visuals.timelinePng', title: $localize`:@@share.visualCommands.timelinePng:Export timeline as PNG…`, category: $localize`:@@share.commands.categoryFile:File`, when: () => !!this.store.timeline(), run: async () => void (await this.visuals.exportTimeline('png')) },
    { id: 'visuals.timelineSvg', title: $localize`:@@share.visualCommands.timelineSvg:Export timeline as SVG…`, category: $localize`:@@share.commands.categoryFile:File`, when: () => !!this.store.timeline(), run: async () => void (await this.visuals.exportTimeline('svg')) },
  ];
}
