import { EnvironmentProviders, inject, makeEnvironmentProviders } from '@angular/core';
import { DEBRIEF_RECORD_MENU_ITEMS, RecordMenuContribution } from '../context/record-actions';
import { WorkspaceStore } from '../core/workspace.store';
import { DEBRIEF_COMMANDS, DEBRIEF_INSPECTOR_SECTIONS, DEBRIEF_VIEWS, DebriefCommand, DebriefInspectorSection, DebriefView } from '../extensions/tokens';
import { UI_STATE_CONTRIBUTIONS } from '../lifecycle/ui-state';
import { anchors, NOTE_IMAGE_VIEW, NoteActions, NOTES_VIEW } from './note-actions';
import { NotesInspectorSection } from './notes-inspector-section';
import { NotesPersistence } from './notes-persistence';

const CATEGORY = () => $localize`:Command palette category|@@notes.category:Notes`;

export function notesViews(): DebriefView[] {
  return [
    { id: NOTES_VIEW, category: 'Analysis', title: $localize`:@@notes.view.notes:Notes`, icon: 'note', defaultPosition: 'right', layout: { area: 'side', min: { width: 320, height: 240 }, pref: { width: 372, height: 400 }, priority: 50 }, component: () => import('./notes-panel').then((m) => m.NotesPanel) },
    { id: NOTE_IMAGE_VIEW, category: 'Analysis', title: $localize`:@@notes.view.image:Image notes`, icon: 'image', defaultPosition: 'center', layout: { area: 'document', min: { width: 420, height: 240 }, pref: { width: 640, height: 400 }, priority: 55 }, component: () => import('./image/note-image-view').then((m) => m.NoteImageView) },
  ];
}

export function notesCommands(actions: NoteActions, workspace: WorkspaceStore): DebriefCommand[] {
  return [
    { id: 'notes.show', title: $localize`:@@notes.cmd.show:Show notes`, category: CATEGORY(), view: NOTES_VIEW, run: (ctx) => ctx.showView(NOTES_VIEW) },
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
    { id: 'notes.image', title: $localize`:@@notes.cmd.image:Mark up an image…`, category: CATEGORY(), view: NOTE_IMAGE_VIEW, run: (ctx) => ctx.showView(NOTE_IMAGE_VIEW) },
  ];
}

/** "Add note…" in the record context menu (the grid's rows, the inspector's "More actions…"). */
export function notesRecordMenu(actions: NoteActions): RecordMenuContribution {
  return (record) => [{ group: 'mark', shortcut: 'Mod+D', label: $localize`:@@notes.menu.add:Add note…`, run: () => void actions.write(anchors.event(record)) }];
}

const INSPECTOR_SECTION: DebriefInspectorSection = { id: 'notes', title: $localize`:@@notes.inspector.title:Notes`, order: 250, component: NotesInspectorSection };

/** One line in `app.config.ts`: the Notes panel and image view, commands, menus, the inspector section, saving. */
export function provideNotes(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: DEBRIEF_VIEWS, multi: true, useFactory: notesViews },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => notesCommands(inject(NoteActions), inject(WorkspaceStore)) },
    { provide: DEBRIEF_RECORD_MENU_ITEMS, multi: true, useFactory: () => notesRecordMenu(inject(NoteActions)) },
    { provide: DEBRIEF_INSPECTOR_SECTIONS, multi: true, useValue: [INSPECTOR_SECTION] },
    { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => [inject(NotesPersistence).contribution] },
  ]);
}
