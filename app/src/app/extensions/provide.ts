import { EnvironmentProviders, makeEnvironmentProviders } from '@angular/core';
import {
  IMT_ACTIVITY_ITEMS,
  IMT_COMMANDS,
  IMT_FIELD_FORMATTERS,
  IMT_INSPECTOR_SECTIONS,
  IMT_VIEWS,
  ImtActivityItem,
  ImtCommand,
  ImtFieldFormatter,
  ImtInspectorSection,
  ImtView,
} from './tokens';

/** Everything a UI plugin can contribute. All parts are optional. */
export interface ImtPlugin {
  views?: ImtView[];
  commands?: ImtCommand[];
  inspectorSections?: ImtInspectorSection[];
  fieldFormatters?: ImtFieldFormatter[];
  /** Buttons in the left activity bar. */
  activityItems?: ImtActivityItem[];
}

/**
 * Registers a UI plugin. One line in `app.config.ts`:
 *
 * ```ts
 * providers: [provideImtPlugin({ views: [MY_VIEW], commands: [MY_COMMAND] })]
 * ```
 */
export function provideImtPlugin(plugin: ImtPlugin): EnvironmentProviders {
  const providers = [];
  if (plugin.views?.length) providers.push({ provide: IMT_VIEWS, multi: true, useValue: plugin.views });
  if (plugin.commands?.length) providers.push({ provide: IMT_COMMANDS, multi: true, useValue: plugin.commands });
  if (plugin.inspectorSections?.length) providers.push({ provide: IMT_INSPECTOR_SECTIONS, multi: true, useValue: plugin.inspectorSections });
  if (plugin.fieldFormatters?.length) providers.push({ provide: IMT_FIELD_FORMATTERS, multi: true, useValue: plugin.fieldFormatters });
  if (plugin.activityItems?.length) providers.push({ provide: IMT_ACTIVITY_ITEMS, multi: true, useValue: plugin.activityItems });
  return makeEnvironmentProviders(providers);
}
