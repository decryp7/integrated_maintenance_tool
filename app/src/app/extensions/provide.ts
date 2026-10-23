import { EnvironmentProviders, makeEnvironmentProviders } from '@angular/core';
import { sanitizeViewLayout } from '../layout-plan/layout-plan';
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

/**
 * Version of the UI plugin contract. 2 added `DebriefView.layout` (area, min, pref, priority; ADR 0056). A plugin
 * that targets version 1 (no `apiVersion`) keeps working: its views get the default layout of their area.
 */
export const UI_PLUGIN_API_VERSION = 2;

/** Everything a UI plugin can contribute. All parts are optional. */
export interface DebriefPlugin {
  /** The UI plugin contract the plugin was written for (default 1). A newer one than this build's is refused. */
  apiVersion?: number;
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
  if ((plugin.apiVersion ?? 1) > UI_PLUGIN_API_VERSION) {
    throw new Error(`This plugin needs UI plugin API ${plugin.apiVersion}, but this Debrief provides ${UI_PLUGIN_API_VERSION}: update Debrief or use an older plugin.`);
  }
  const providers = [];
  // A view's layout is parsed once here: bounded numbers, min <= pref, a known area; an unusable one falls back to its area's default.
  const views = plugin.views?.map((v) => {
    const { layout: raw, ...rest } = v;
    const layout = sanitizeViewLayout(raw);
    return layout ? { ...rest, layout } : (rest as DebriefView);
  });
  if (views?.length) providers.push({ provide: DEBRIEF_VIEWS, multi: true, useValue: views });
  if (plugin.commands?.length) providers.push({ provide: DEBRIEF_COMMANDS, multi: true, useValue: plugin.commands });
  if (plugin.inspectorSections?.length) providers.push({ provide: DEBRIEF_INSPECTOR_SECTIONS, multi: true, useValue: plugin.inspectorSections });
  if (plugin.fieldFormatters?.length) providers.push({ provide: DEBRIEF_FIELD_FORMATTERS, multi: true, useValue: plugin.fieldFormatters });
  if (plugin.activityItems?.length) providers.push({ provide: DEBRIEF_ACTIVITY_ITEMS, multi: true, useValue: plugin.activityItems });
  return makeEnvironmentProviders(providers);
}
