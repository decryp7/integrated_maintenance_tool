import { EnvironmentProviders, inject, makeEnvironmentProviders } from '@angular/core';
import { IMT_RECORD_MENU_ITEMS, RecordMenuContribution } from '../context/record-actions';
import { WorkspaceStore } from '../core/workspace.store';
import { IMT_COMMANDS, IMT_INSPECTOR_SECTIONS, IMT_VIEWS, ImtCommand, ImtInspectorSection, ImtView } from '../extensions/tokens';
import { UI_STATE_CONTRIBUTIONS } from '../lifecycle/ui-state';
import { anchors, NOTE_IMAGE_VIEW, NoteActions, NOTES_VIEW } from './note-actions';
import { NotesInspectorSection } from './notes-inspector-section';
import { NotesPersistence } from './notes-persistence';

const CATEGORY = () => $localize`:Command palette category|@@notes.category:Notes`;

export function notesViews(): ImtView[] {
  return [
    { id: NOTES_VIEW, category: 'Analysis', title: $localize`:@@notes.view.notes:Notes`, icon: 'note', defaultPosition: 'right', component: () => import('./notes-panel').then((m) => m.NotesPanel) },
    { id: NOTE_IMAGE_VIEW, category: 'Analysis', title: $localize`:@@notes.view.image:Image notes`, icon: 'image', defaultPosition: 'center', component: () => import('./image/note-image-view').then((m) => m.NoteImageView) },
  ];
}

export function notesCommands(actions: NoteActions, workspace: WorkspaceStore): ImtCommand[] {
  return [
    { id: 'notes.show', title: $localize`:@@notes.cmd.show:Show notes`, category: CATEGORY(), run: (ctx) => ctx.showView(NOTES_VIEW) },
    {
      id: 'notes.addSelected',
      title: $localize`:@@notes.cmd.addSelected:Add note to the selected event…`,
      category: CATEGORY(),
      keybinding: 'Mod+D',
      when: () => !!workspace.selected(),
      run: () => void actions.writeOnSelected(),
    },
    { id: 'notes.addRange', title: $localize`:@@notes.cmd.addRange:Add note to the time range…`, category: CATEGORY(), when: () => workspace.hasRange(), run: () => void actions.writeOnRange() },
    { id: 'notes.addProject', title: $localize`:@@notes.cmd.addProject:Add note to the project…`, category: CATEGORY(), run: () => void actions.write(anchors.project()) },
    { id: 'notes.image', title: $localize`:@@notes.cmd.image:Mark up an image…`, category: CATEGORY(), run: (ctx) => ctx.showView(NOTE_IMAGE_VIEW) },
  ];
}

/** "Add note…" in the record context menu (the grid's rows, the inspector's "More actions…"). */
export function notesRecordMenu(actions: NoteActions): RecordMenuContribution {
  return (record) => [{ label: $localize`:@@notes.menu.add:Add note…`, run: () => void actions.write(anchors.event(record)) }];
}

const INSPECTOR_SECTION: ImtInspectorSection = { id: 'notes', title: $localize`:@@notes.inspector.title:Notes`, order: 250, component: NotesInspectorSection };

/** One line in `app.config.ts`: the Notes panel and image view, commands, menus, the inspector section, saving. */
export function provideNotes(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: IMT_VIEWS, multi: true, useFactory: notesViews },
    { provide: IMT_COMMANDS, multi: true, useFactory: () => notesCommands(inject(NoteActions), inject(WorkspaceStore)) },
    { provide: IMT_RECORD_MENU_ITEMS, multi: true, useFactory: () => notesRecordMenu(inject(NoteActions)) },
    { provide: IMT_INSPECTOR_SECTIONS, multi: true, useValue: [INSPECTOR_SECTION] },
    { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => [inject(NotesPersistence).contribution] },
  ]);
}
