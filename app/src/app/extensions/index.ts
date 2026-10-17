/**
 * Public API for IMT UI plugins.
 *
 * A plugin is a set of contributions registered in `app.config.ts` with one call:
 *
 * ```ts
 * provideImtPlugin({ views, commands, inspectorSections, fieldFormatters })
 * ```
 *
 * - `ImtView`            dockable panel, loaded lazily, offered while `when(summary)` holds
 * - `ImtCommand`         palette / menu action with optional key binding; receives a `CommandContext`
 * - `ImtInspectorSection` extra block in the event inspector (component with a `record` input)
 * - `ImtFieldFormatter`  display formatting for fields in the grid and inspector
 * - `ImtEvents`          read-only streams of workspace / query / selection / profile changes
 *
 * Plugins never touch the stores: they observe through `ImtEvents` and act through `CommandContext`.
 */
export { CommandRegistry, COMMAND_CONTEXT } from './command-registry';
export { WorkspaceCommandContext } from './command-context';
export { ImtEvents, StoreImtEvents } from './events';
export { EVENT_ID_FORMATTER, FieldFormatters, findFormatter } from './field-formatters';
export { fuzzyFilter, fuzzyScore } from './fuzzy';
export { InspectorSections } from './inspector-sections';
export { formatKeybinding, matchesKeybinding } from './keybinding';
export { provideImtPlugin, type ImtPlugin } from './provide';
export {
  flattenContributions,
  IMT_COMMANDS,
  IMT_FIELD_FORMATTERS,
  IMT_INSPECTOR_SECTIONS,
  IMT_VIEWS,
  type CommandContext,
  type ImtCommand,
  type ImtFieldFormatter,
  type ImtInspectorSection,
  type ImtView,
  type ViewPosition,
} from './tokens';
export { isViewAvailable, ViewRegistry } from './view-registry';
