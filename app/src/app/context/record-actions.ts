import { inject, Injectable } from '@angular/core';
import { RecordView } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { ContextMenuService, MenuEntry } from '../ui/context-menu.service';
import { DialogService } from '../ui/dialog.service';
import { DropDialog } from './drop-dialog';
import { ExtractDialog } from './extract-dialog';
import { MarkDialog } from './mark-dialog';
import { MaskDialog } from './mask-dialog';
import { fieldValueConditions, likeThisConditions, recordFieldText } from './rule-builders';

/** A field and value the user pointed at (inspector property row). */
export interface FieldRef {
  name: string;
  value: string;
}

/** Text the user selected in the inspector message (offsets into `record.message`). */
export interface MessageSelection {
  text: string;
  start: number;
  end: number;
}

export interface ActionTarget {
  field?: FieldRef;
  selection?: MessageSelection | null;
}

const LABEL_MAX = 40;

/**
 * The actions offered on a record or one of its values: filter, and the "teach IMT from this event"
 * dialogs (drop, mask, extract, mark). Builds menu entries; the grid and inspector only open the menu.
 */
@Injectable({ providedIn: 'root' })
export class RecordActions {
  private readonly store = inject(WorkspaceStore);
  private readonly dialogs = inject(DialogService);
  private readonly menu = inject(ContextMenuService);

  itemsFor(record: RecordView, target: ActionTarget = {}): MenuEntry[] {
    const items: MenuEntry[] = [];
    const filterField = target.field ?? this.contextField(record);
    if (filterField) {
      const { name, value } = filterField;
      items.push(
        { label: `Filter: only this ${name}`, run: () => this.store.addFilter({ field: name, op: 'eq', value }) },
        { label: `Exclude this ${name}`, run: () => this.store.addFilter({ field: name, op: 'ne', value }) },
      );
    }
    items.push(
      { label: 'Drop events like this (pipeline)…', run: () => this.drop(record, target.field) },
      { label: 'Mark events like this…', run: () => this.mark(record, target.field) },
    );
    const sel = target.selection;
    if (sel?.text.trim()) {
      items.push({ label: 'Mask in message…', run: () => this.mask(record, sel) }, { label: 'Extract value…', run: () => this.extract(record, sel) });
    }
    return items;
  }

  /** Opens the menu next to the pointer (mouse) or the focused element (keyboard). */
  openFor(event: MouseEvent | KeyboardEvent, record: RecordView, target: ActionTarget = {}): void {
    const returnFocus = event.target as HTMLElement | null;
    if (event.type === 'contextmenu' && event instanceof MouseEvent && (event.clientX || event.clientY)) {
      this.menu.open(event.clientX, event.clientY, this.itemsFor(record, target), returnFocus);
    } else {
      this.openAtElement(event.currentTarget instanceof HTMLElement ? event.currentTarget : returnFocus, record, target, returnFocus);
    }
  }

  /** Opens the menu below `anchor` (keyboard access); focus returns to `returnFocus` when it closes. */
  openAtElement(anchor: HTMLElement | null, record: RecordView, target: ActionTarget = {}, returnFocus: HTMLElement | null = anchor): void {
    const r = anchor?.getBoundingClientRect();
    this.menu.open(r ? r.left + 16 : 40, r ? r.bottom : 40, this.itemsFor(record, target), returnFocus);
  }

  drop(record: RecordView, field?: FieldRef): void {
    const initial = field ? fieldValueConditions(field.name, field.value) : likeThisConditions(record);
    this.dialogs.open(DropDialog, { title: 'Drop events like this', inputs: { initial }, width: 640 });
  }

  mark(record: RecordView, field?: FieldRef): void {
    const initial = field ? fieldValueConditions(field.name, field.value) : likeThisConditions(record);
    const basis = field ? `${field.name} ${field.value}` : (record.template ?? record.message);
    const suggestedLabel = basis.length > LABEL_MAX ? `${basis.slice(0, LABEL_MAX).trim()}…` : basis;
    this.dialogs.open(MarkDialog, { title: 'Mark events like this', inputs: { initial, suggestedLabel }, width: 640 });
  }

  mask(record: RecordView, selection: MessageSelection): void {
    this.dialogs.open(MaskDialog, { title: 'Mask in message', inputs: { record, selected: selection.text }, width: 640 });
  }

  extract(record: RecordView, selection: MessageSelection): void {
    this.dialogs.open(ExtractDialog, { title: 'Extract value', inputs: { record, start: selection.start, end: selection.end }, width: 680 });
  }

  /** "Filter only this logger" is the most useful row-level filter when the record has a SourceContext. */
  private contextField(record: RecordView): FieldRef | null {
    const value = recordFieldText(record, 'SourceContext');
    return value ? { name: 'SourceContext', value } : null;
  }
}
