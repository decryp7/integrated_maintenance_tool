import { inject, Injectable } from '@angular/core';
import { WorkspaceStore } from '../../core/workspace.store';
import { ImtCommand } from '../../extensions/tokens';
import { VisualExportService } from './visual-export.service';

/** Commands for chart and timeline images (charts export from their card's buttons). */
@Injectable({ providedIn: 'root' })
export class VisualCommands {
  private readonly visuals = inject(VisualExportService);
  private readonly store = inject(WorkspaceStore);

  readonly commands: ImtCommand[] = [
    { id: 'visuals.timelinePng', title: 'Export timeline as PNG…', category: 'File', when: () => !!this.store.timeline(), run: async () => void (await this.visuals.exportTimeline('png')) },
    { id: 'visuals.timelineSvg', title: 'Export timeline as SVG…', category: 'File', when: () => !!this.store.timeline(), run: async () => void (await this.visuals.exportTimeline('svg')) },
  ];
}
