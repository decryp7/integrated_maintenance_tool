import { inject, Injectable, InjectionToken } from '@angular/core';
import { RecordView } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { GridSelection } from '../grid/grid-selection';
import { ContextMenuService, MenuEntry } from '../ui/context-menu.service';
import { DialogService } from '../ui/dialog.service';
import { DropDialog } from './drop-dialog';
import { ExtractDialog } from './extract-dialog';
import { InferDialog } from '../markers/regex/infer-dialog';
import { proposeHighlight } from '../markers/highlight';
import { MarkDialog } from './mark-dialog';
import { MaskDialog } from './mask-dialog';
import { fieldValueConditions, likeThisConditions, recordFieldText } from './rule-builders';
import { SourceFileActions } from '../sources/source-file-actions';
import { copyText } from '../tools/clipboard';

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
  /** All records the user selected (grid multi-select), in order; the actions on several lines use them. Defaults to the record alone. */
  records?: readonly RecordView[];
  /** Show Mask and Extract even without a selection, disabled with the reason (the inspector, where the message is). */
  explainTextRules?: boolean;
}

const LABEL_MAX = 40;

/**
 * What the record menu's Copy puts on the clipboard (ADR 0028): the text the user selected (in the inspector message,
 * else anywhere in the window), otherwise the messages of the selected records, one per line. Pure.
 */
export function copyTextFor(record: RecordView, target: ActionTarget, windowSelection: string): string {
  const selected = target.selection?.text.trim() ? target.selection.text : windowSelection;
  if (selected.trim()) return selected;
  return (target.records?.length ? target.records : [record]).map((r) => r.message).join('\n');
}

/** The order of the record menu's own groups; contributed groups follow in their order. */
const GROUP_ORDER = ['copy', 'filter', 'mark', 'open', 'rules'];

/** Puts the entries of each group together: known groups in `GROUP_ORDER`, then the rest as they first appear. Pure. */
export function groupMenu(items: readonly MenuEntry[]): MenuEntry[] {
  const groups: string[] = [...GROUP_ORDER];
  for (const e of items) if (!groups.includes(e.group ?? '')) groups.push(e.group ?? '');
  return groups.flatMap((g) => items.filter((e) => (e.group ?? '') === g));
}

/**
 * Extra entries for the record context menu, contributed by features (multi provider; each returns
 * the entries for the record and the user's target).
 */
export type RecordMenuContribution = (record: RecordView, target: ActionTarget) => MenuEntry[];
export const DEBRIEF_RECORD_MENU_ITEMS = new InjectionToken<readonly RecordMenuContribution[]>('DEBRIEF_RECORD_MENU_ITEMS');

/**
 * The actions offered on a record or one of its values: filter, the "teach Debrief from this event"
 * dialogs (drop, mask, extract, mark), and opening its file outside Debrief. Builds menu entries; the grid and inspector only open the menu.
 */
@Injectable({ providedIn: 'root' })
export class RecordActions {
  private readonly store = inject(WorkspaceStore);
  private readonly dialogs = inject(DialogService);
  private readonly menu = inject(ContextMenuService);
  private readonly gridSelection = inject(GridSelection);
  private readonly files = inject(SourceFileActions);
  private readonly contributions = inject(DEBRIEF_RECORD_MENU_ITEMS, { optional: true }) ?? [];

  /**
   * The menu in groups (separated in the menu): Copy first, then filter, mark and note, open the file, the "teach Debrief"
   * rules, then each contribution's own group (AI, web, report) in registration order.
   */
  itemsFor(record: RecordView, target: ActionTarget = {}): MenuEntry[] {
    const items: MenuEntry[] = [];
    const text = copyTextFor(record, target, typeof window === 'undefined' ? '' : (window.getSelection()?.toString() ?? ''));
    items.push({ group: 'copy', label: $localize`:Context menu item that copies the selected text, else the messages@@context.recordActions.menu.copy:Copy`, run: () => void copyText(text) });
    const filterField = target.field ?? this.contextField(record);
    if (filterField) {
      const { name, value } = filterField;
      items.push(
        { group: 'filter', label: $localize`:@@context.recordActions.onlyThis:Only this ${name}:name:`, run: () => this.store.addFilter({ field: name, op: 'eq', value }) },
        { group: 'filter', label: $localize`:@@context.recordActions.exclude:Exclude this ${name}:name:`, run: () => this.store.addFilter({ field: name, op: 'ne', value }) },
      );
    }
    items.push({ group: 'mark', label: $localize`:@@context.recordActions.menu.highlight:Highlight lines…`, run: () => this.highlight(record, target.selection?.text) });
    items.push(...this.files.itemsForRecord(record).map((e) => ({ ...e, group: 'open' })));
    items.push(
      { group: 'rules', label: $localize`:@@context.recordActions.menu.mark:Mark events like this…`, run: () => this.mark(record, target.field) },
      { group: 'rules', label: $localize`:@@context.recordActions.menu.drop:Drop events like this (pipeline)…`, run: () => this.drop(record, target.field) },
      { group: 'rules', label: $localize`:@@context.recordActions.menu.infer:Infer pattern from lines…`, run: () => this.infer(target.records?.length ? target.records : [record]) },
    );
    const sel = target.selection;
    if (sel?.text.trim()) {
      items.push(
        { group: 'rules', label: $localize`:@@context.recordActions.menu.mask:Mask in message…`, run: () => this.mask(record, sel) },
        { group: 'rules', label: $localize`:@@context.recordActions.menu.extract:Extract value…`, run: () => this.extract(record, sel) },
      );
    } else if (target.explainTextRules) {
      const hint = $localize`:Why Mask and Extract are unavailable@@context.recordActions.menu.selectFirst:Select text in the message first`;
      items.push(
        { group: 'rules', label: $localize`:@@context.recordActions.menu.mask:Mask in message…`, run: () => undefined, disabled: true, hint },
        { group: 'rules', label: $localize`:@@context.recordActions.menu.extract:Extract value…`, run: () => undefined, disabled: true, hint },
      );
    }
    this.contributions.forEach((contribute, i) => items.push(...contribute(record, target).map((e) => ({ ...e, group: e.group ?? `contribution-${i}` }))));
    return groupMenu(items);
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
    this.dialogs.open(DropDialog, { title: $localize`:@@context.recordActions.drop.title:Drop events like this`, inputs: { initial }, width: 640 });
  }

  mark(record: RecordView, field?: FieldRef): void {
    const initial = field ? fieldValueConditions(field.name, field.value) : likeThisConditions(record);
    const basis = field ? `${field.name} ${field.value}` : (record.template ?? record.message);
    const suggestedLabel = basis.length > LABEL_MAX ? `${basis.slice(0, LABEL_MAX).trim()}…` : basis;
    this.dialogs.open(MarkDialog, { title: $localize`:@@context.recordActions.mark.title:Mark events like this`, inputs: { initial, suggestedLabel }, width: 760 });
  }

  /** "Highlight lines…": a marker rule prefilled from the selected text (as a regex), the template or the message. */
  highlight(record: RecordView, selectedText?: string | null): void {
    const p = proposeHighlight(record, selectedText);
    const hint = {
      selection: $localize`:@@context.recordActions.highlight.hintSelection:The selected text became a regex (numbers and ids generalised). Edit it below; the preview shows what it highlights.`,
      template: $localize`:@@context.recordActions.highlight.hintTemplate:Highlights every event with this message template.`,
      message: $localize`:@@context.recordActions.highlight.hintMessage:Highlights events whose message looks like this one (numbers generalised).`,
    }[p.basis];
    this.dialogs.open(MarkDialog, {
      title: $localize`:@@context.recordActions.highlight.title:Highlight lines`,
      inputs: { initial: p.conditions, suggestedLabel: p.label, submitLabel: $localize`:@@context.recordActions.highlight.submit:Add highlight`, hint },
      width: 760,
    });
  }

  /** "Infer pattern from lines…": derives one regex from the messages of these records. */
  infer(records: readonly RecordView[]): void {
    this.dialogs.open(InferDialog, { title: $localize`:@@context.recordActions.infer.title:Infer pattern from lines`, inputs: { lines: records.map((r) => r.message) }, width: 780 });
  }

  /**
   * "Infer pattern from selected lines…" (command palette): the grid's multi-selection, else the record selected
   * elsewhere (timeline, notes). Returns false, without opening anything, when nothing is selected.
   */
  inferSelected(): boolean {
    const records = this.gridSelection.recordsOr(this.store.selected());
    if (!records.length) return false;
    this.infer(records);
    return true;
  }

  mask(record: RecordView, selection: MessageSelection): void {
    this.dialogs.open(MaskDialog, { title: $localize`:@@context.recordActions.mask.title:Mask in message`, inputs: { record, selected: selection.text }, width: 640 });
  }

  extract(record: RecordView, selection: MessageSelection): void {
    this.dialogs.open(ExtractDialog, { title: $localize`:@@context.recordActions.extract.title:Extract value`, inputs: { record, start: selection.start, end: selection.end }, width: 680 });
  }

  /** "Filter only this logger" is the most useful row-level filter when the record has a SourceContext. */
  private contextField(record: RecordView): FieldRef | null {
    const value = recordFieldText(record, 'SourceContext');
    return value ? { name: 'SourceContext', value } : null;
  }
}
