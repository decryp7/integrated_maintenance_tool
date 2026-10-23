import { EnvironmentProviders, inject, makeEnvironmentProviders, provideAppInitializer } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { DEBRIEF_ACTIVITY_ITEMS, DEBRIEF_COMMANDS, DEBRIEF_FIELD_FORMATTERS, DEBRIEF_VIEWS, DebriefView } from '../extensions/tokens';
import { EVENT_ID_FORMATTER } from '../extensions/field-formatters';
import { builtinActivityItems } from './builtin-activity';
import { BuiltinCommands } from './builtin-commands';
import { BUILTIN_VIEWS } from './builtin-views';
import { RegionNavigator } from '../shell/regions';
import { EditCommands } from '../shell/edit-commands';
import { PasteCommands } from '../sources/paste/paste-commands';
import { startTextDrops } from '../sources/paste/paste-drop';

/** Registers the built-in views, commands and formatters through the same extension points plugins use. */
/** `views` can be replaced in tests (e.g. to stub heavy panels). */
export function provideDebriefBuiltins(views: readonly DebriefView[] = BUILTIN_VIEWS): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: DEBRIEF_VIEWS, multi: true, useValue: views },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(BuiltinCommands).commands },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(RegionNavigator).commands },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(EditCommands).commands },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(PasteCommands).commands },
    provideAppInitializer(startTextDrops),
    { provide: DEBRIEF_FIELD_FORMATTERS, multi: true, useValue: [EVENT_ID_FORMATTER] },
    { provide: DEBRIEF_ACTIVITY_ITEMS, multi: true, useFactory: () => builtinActivityItems(inject(WorkspaceStore)) },
  ]);
}
