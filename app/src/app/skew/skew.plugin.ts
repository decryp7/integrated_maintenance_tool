import { EnvironmentProviders, inject, makeEnvironmentProviders } from '@angular/core';
import { provideSkewBackend } from '../core/skew-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { DEBRIEF_COMMANDS, DEBRIEF_INSPECTOR_SECTIONS, DebriefCommand, DebriefInspectorSection } from '../extensions/tokens';
import { ClockDialogService, ClockSection } from './clock-section';
import { ClockSkewStore } from './clock-skew.store';

/** The inspector section offering "Use as clock reference" and alignment (timed events only). */
export const CLOCK_SECTION: DebriefInspectorSection = { id: 'clock', title: $localize`:@@skew.plugin.section:Clock`, order: 150, component: ClockSection, when: (r) => r.ts !== null };

/** Palette commands for clock-skew correction. */
export function clockCommands(): DebriefCommand[] {
  const store = inject(WorkspaceStore);
  const skew = inject(ClockSkewStore);
  const dialog = inject(ClockDialogService);
  return [
    {
      id: 'skew.adjust',
      title: $localize`:@@skew.plugin.adjust:Adjust clock of the selected event's source…`,
      category: $localize`:@@skew.plugin.category:Events`,
      when: () => store.selected() !== null,
      run: () => {
        const r = store.selected();
        if (r) dialog.open(r.source, r.sourceName);
      },
    },
    {
      id: 'skew.reference',
      title: $localize`:@@skew.plugin.reference:Use selected event as clock reference`,
      category: $localize`:@@skew.plugin.category:Events`,
      when: () => store.selected()?.ts != null,
      run: () => void skew.setReference(store.selected()),
    },
  ];
}

/**
 * Clock-skew correction: per-source offsets (Sources › Adjust clock…), the inspector's clock section and
 * palette commands. One line in `app.config.ts`.
 */
export function provideClockSkew(): EnvironmentProviders {
  return makeEnvironmentProviders([
    provideSkewBackend(),
    { provide: DEBRIEF_INSPECTOR_SECTIONS, multi: true, useValue: [CLOCK_SECTION] },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: clockCommands },
  ]);
}
