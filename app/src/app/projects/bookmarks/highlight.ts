import { computed } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { RecordView } from '../../core/api.types';
import { HighlightOp, HighlightRule } from '../../core/tools.types';
import { newId } from './ids';

export const HIGHLIGHT_OPS: { id: HighlightOp; label: string; needsValue: boolean }[] = [
  { id: 'eq', label: 'equals', needsValue: true },
  { id: 'ne', label: 'does not equal', needsValue: true },
  { id: 'contains', label: 'contains', needsValue: true },
  { id: 'notContains', label: 'does not contain', needsValue: true },
  { id: 'gt', label: 'is greater than', needsValue: true },
  { id: 'lt', label: 'is less than', needsValue: true },
  { id: 'exists', label: 'exists', needsValue: false },
  { id: 'missing', label: 'is missing', needsValue: false },
];

/** Colours offered for rows (distinguishable and readable as a tint behind text on both themes). */
export const HIGHLIGHT_COLORS = ['#f2545b', '#e9ae3f', '#3fbf7f', '#4c8dff', '#b07cf2', '#ff4fa3'];

/** A record's value for a field or pseudo-field (`@level`, `@message`, `@exception`, `@source`). */
export function fieldValue(record: RecordView, field: string): unknown {
  switch (field) {
    case '@level':
      return record.level;
    case '@message':
      return record.message;
    case '@exception':
      return record.exception ?? undefined;
    case '@source':
      return record.sourceName;
    default:
      return record.fields[field];
  }
}

/** Whether a rule's condition holds for a record. Purely visual: it decides a row's colour only. */
export function matchesRule(rule: HighlightRule, record: RecordView): boolean {
  const value = fieldValue(record, rule.field);
  const present = value !== undefined && value !== null && value !== '';
  if (rule.op === 'exists') return present;
  if (rule.op === 'missing') return !present;
  if (!present) return rule.op === 'ne' || rule.op === 'notContains';
  const text = String(value);
  switch (rule.op) {
    case 'eq':
      return text === rule.value;
    case 'ne':
      return text !== rule.value;
    case 'contains':
      return text.toLowerCase().includes(rule.value.toLowerCase());
    case 'notContains':
      return !text.toLowerCase().includes(rule.value.toLowerCase());
    case 'gt':
    case 'lt': {
      const a = Number(text);
      const b = Number(rule.value);
      if (rule.value.trim() === '' || Number.isNaN(a) || Number.isNaN(b)) return false;
      return rule.op === 'gt' ? a > b : a < b;
    }
  }
}

const OPS = new Set<string>(HIGHLIGHT_OPS.map((o) => o.id));

export function parseHighlightRules(value: unknown): HighlightRule[] {
  if (!Array.isArray(value)) return [];
  return (value as Partial<HighlightRule>[])
    .filter((r) => !!r && typeof r.id === 'string' && typeof r.field === 'string' && typeof r.op === 'string' && OPS.has(r.op) && typeof r.color === 'string')
    .map((r) => ({ id: r.id!, field: r.field!, op: r.op!, value: typeof r.value === 'string' ? r.value : '', color: r.color!, enabled: r.enabled !== false }));
}

interface HighlightState {
  items: HighlightRule[];
}

/**
 * Highlight rules (field condition -> row colour), saved in the project.
 *
 * Hook for the events grid: read `colorFor(record)` while rendering a row (it reads signals, so
 * rows repaint when rules change). The first matching enabled rule wins. `rules` is the published
 * signal of the enabled rules in order.
 */
export const HighlightRules = signalStore(
  { providedIn: 'root' },
  withState<HighlightState>({ items: [] }),
  withComputed((store) => ({
    /** Enabled rules in order. */
    rules: computed(() => store.items().filter((r) => r.enabled)),
  })),
  withMethods((store) => ({
    add(rule: Omit<HighlightRule, 'id' | 'enabled'>): void {
      patchState(store, (s) => ({ items: [...s.items, { ...rule, id: newId('hl'), enabled: true }] }));
    },
    update: (id: string, patch: Partial<Omit<HighlightRule, 'id'>>) => patchState(store, (s) => ({ items: s.items.map((r) => (r.id === id ? { ...r, ...patch } : r)) })),
    remove: (id: string) => patchState(store, (s) => ({ items: s.items.filter((r) => r.id !== id) })),
    replace: (items: HighlightRule[]) => patchState(store, { items }),
    /** The row colour (CSS colour) for a record, or null when no rule matches. */
    colorFor(record: RecordView): string | null {
      return store.rules().find((r) => matchesRule(r, record))?.color ?? null;
    },
  })),
);
