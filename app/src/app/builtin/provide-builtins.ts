import { EnvironmentProviders, inject, makeEnvironmentProviders } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { IMT_ACTIVITY_ITEMS, IMT_COMMANDS, IMT_FIELD_FORMATTERS, IMT_VIEWS, ImtView } from '../extensions/tokens';
import { EVENT_ID_FORMATTER } from '../extensions/field-formatters';
import { builtinActivityItems } from './builtin-activity';
import { BuiltinCommands } from './builtin-commands';
import { BUILTIN_VIEWS } from './builtin-views';

/** Registers the built-in views, commands and formatters through the same extension points plugins use. */
/** `views` can be replaced in tests (e.g. to stub heavy panels). */
export function provideImtBuiltins(views: readonly ImtView[] = BUILTIN_VIEWS): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: IMT_VIEWS, multi: true, useValue: views },
    { provide: IMT_COMMANDS, multi: true, useFactory: () => inject(BuiltinCommands).commands },
    { provide: IMT_FIELD_FORMATTERS, multi: true, useValue: [EVENT_ID_FORMATTER] },
    { provide: IMT_ACTIVITY_ITEMS, multi: true, useFactory: () => builtinActivityItems(inject(WorkspaceStore)) },
  ]);
}
