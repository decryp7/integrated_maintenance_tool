import { EnvironmentProviders, makeEnvironmentProviders } from '@angular/core';
import {
  DEBRIEF_ACTIVITY_ITEMS,
  DEBRIEF_COMMANDS,
  DEBRIEF_FIELD_FORMATTERS,
  DEBRIEF_INSPECTOR_SECTIONS,
  DEBRIEF_VIEWS,
  DebriefActivityItem,
  DebriefCommand,
  DebriefFieldFormatter,
  DebriefInspectorSection,
  DebriefView,
} from './tokens';

/** Everything a UI plugin can contribute. All parts are optional. */
export interface DebriefPlugin {
  views?: DebriefView[];
  commands?: DebriefCommand[];
  inspectorSections?: DebriefInspectorSection[];
  fieldFormatters?: DebriefFieldFormatter[];
  /** Buttons in the left activity bar. */
  activityItems?: DebriefActivityItem[];
}

/**
 * Registers a UI plugin. One line in `app.config.ts`:
 *
 * ```ts
 * providers: [provideDebriefPlugin({ views: [MY_VIEW], commands: [MY_COMMAND] })]
 * ```
 */
export function provideDebriefPlugin(plugin: DebriefPlugin): EnvironmentProviders {
  const providers = [];
  if (plugin.views?.length) providers.push({ provide: DEBRIEF_VIEWS, multi: true, useValue: plugin.views });
  if (plugin.commands?.length) providers.push({ provide: DEBRIEF_COMMANDS, multi: true, useValue: plugin.commands });
  if (plugin.inspectorSections?.length) providers.push({ provide: DEBRIEF_INSPECTOR_SECTIONS, multi: true, useValue: plugin.inspectorSections });
  if (plugin.fieldFormatters?.length) providers.push({ provide: DEBRIEF_FIELD_FORMATTERS, multi: true, useValue: plugin.fieldFormatters });
  if (plugin.activityItems?.length) providers.push({ provide: DEBRIEF_ACTIVITY_ITEMS, multi: true, useValue: plugin.activityItems });
  return makeEnvironmentProviders(providers);
}
