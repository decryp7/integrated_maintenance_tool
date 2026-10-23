/** Pure operations on the marker rules of a profile model. */
import { Condition, ConditionOp, ProfileModel } from '../core/profile.model';
import { MarkerRuleX, PendingIcon, RuleRow } from './markers.types';

/** Profile key holding disabled rules (the engine ignores unknown keys). */
export const DISABLED_KEY = 'debrief_disabled_markers';
/** Position a disabled rule had in the priority order, so enabling puts it back. */
const INDEX_KEY = 'debrief_index';

const clone = <T>(v: T): T => structuredClone(v);

/** All rules in priority order: the active `[[marker]]` rules with disabled ones at their old positions. */
export function readRows(model: ProfileModel): RuleRow[] {
  const rows: RuleRow[] = ((model.marker ?? []) as MarkerRuleX[]).map((rule) => ({ rule: clone(rule), enabled: true }));
  const disabled = ((model[DISABLED_KEY] as MarkerRuleX[] | undefined) ?? []).map((r) => {
    const { [INDEX_KEY]: idx, ...rule } = clone(r) as MarkerRuleX & { [INDEX_KEY]?: unknown };
    return { rule: rule as MarkerRuleX, at: typeof idx === 'number' ? idx : Number.MAX_SAFE_INTEGER };
  });
  disabled.sort((a, b) => a.at - b.at);
  for (const d of disabled) rows.splice(Math.min(d.at, rows.length), 0, { rule: d.rule, enabled: false });
  return rows;
}

/** Writes rows back: active rules become `[[marker]]`, disabled ones go to `debrief_disabled_markers`. */
export function writeRows(model: ProfileModel, rows: readonly RuleRow[]): void {
  model.marker = rows.filter((r) => r.enabled).map((r) => clone(r.rule));
  const disabled = rows.flatMap((r, i) => (r.enabled ? [] : [{ ...clone(r.rule), [INDEX_KEY]: i }]));
  if (disabled.length) model[DISABLED_KEY] = disabled;
  else delete model[DISABLED_KEY];
}

export function moveRow<T>(rows: readonly T[], from: number, to: number): T[] {
  const out = [...rows];
  if (from < 0 || from >= out.length) return out;
  const moved = out.splice(from, 1);
  out.splice(Math.max(0, Math.min(to, out.length)), 0, ...moved);
  return out;
}

/** The badges already used, case-insensitively. */
function usedBadges(rows: readonly RuleRow[]): Set<string> {
  return new Set(rows.map((r) => r.rule.badge?.toLowerCase()).filter((b): b is string => !!b));
}

/** Next unused badge: 1..9, then A..Z, then 10..99. Empty string when everything is taken. */
export function nextFreeBadge(rows: readonly RuleRow[]): string {
  const used = usedBadges(rows);
  const candidates = [
    ...Array.from({ length: 9 }, (_, i) => String(i + 1)),
    ...Array.from({ length: 26 }, (_, i) => String.fromCharCode(65 + i)),
    ...Array.from({ length: 90 }, (_, i) => String(i + 10)),
  ];
  return candidates.find((c) => !used.has(c.toLowerCase())) ?? '';
}

/** A marker kind id not used by any rule: `base`, `base-2`, ... */
export function uniqueKind(rows: readonly RuleRow[], base: string, own?: string): string {
  const taken = new Set(rows.map((r) => r.rule.kind).filter((k) => k !== own));
  let kind = base;
  for (let i = 2; taken.has(kind); i++) kind = `${base}-${i}`;
  return kind;
}

const FIELD_NAMES: Record<string, string> = {
  '@message': $localize`:Name of an event field in a rule description|@@markers.rules.field.message:message`,
  '@level': $localize`:Name of an event field in a rule description|@@markers.rules.field.level:level`,
  '@exception': $localize`:Name of an event field in a rule description|@@markers.rules.field.exception:exception`,
  '@template': $localize`:Name of an event field in a rule description|@@markers.rules.field.template:message template`,
  '@source': $localize`:Name of an event field in a rule description|@@markers.rules.field.source:source file`,
  '@line': $localize`:Name of an event field in a rule description|@@markers.rules.field.line:line number`,
};

const OP_TEXT: Record<ConditionOp, string> = {
  eq: $localize`:Condition operator in a rule description|@@markers.rules.op.eq:is`,
  ne: $localize`:Condition operator in a rule description|@@markers.rules.op.ne:is not`,
  contains: $localize`:Condition operator in a rule description|@@markers.rules.op.contains:contains`,
  notContains: $localize`:Condition operator in a rule description|@@markers.rules.op.notContains:does not contain`,
  matches: $localize`:Condition operator in a rule description|@@markers.rules.op.matches:matches regex`,
  exists: $localize`:Condition operator in a rule description|@@markers.rules.op.exists:is present`,
  missing: $localize`:Condition operator in a rule description|@@markers.rules.op.missing:is absent`,
  gt: $localize`:Condition operator in a rule description|@@markers.rules.op.gt:is greater than`,
  lt: $localize`:Condition operator in a rule description|@@markers.rules.op.lt:is less than`,
  in: $localize`:Condition operator in a rule description|@@markers.rules.op.in:is one of`,
};

export function describeCondition(c: Condition): string {
  const field = FIELD_NAMES[c.field] ?? c.field;
  const op = OP_TEXT[c.op] ?? c.op;
  return c.value === undefined || c.op === 'exists' || c.op === 'missing'
    ? $localize`:@@markers.rules.describe:${field}:field: ${op}:op:`
    : $localize`:@@markers.rules.describeValue:${field}:field: ${op}:op: "${c.value}:value:"`;
}

/** Plain-English summary of a rule's conditions. */
export function describeWhen(when: readonly Condition[] | undefined): string {
  return when?.length ? when.map(describeCondition).join($localize`:Joins two rule conditions|@@markers.rules.and: and `) : $localize`:@@markers.rules.everyEvent:every event`;
}

/** Names of custom icons that rules (active or disabled) still use. */
function usedIcons(rows: readonly RuleRow[]): Set<string> {
  return new Set(rows.map((r) => r.rule.icon).filter((i): i is string => !!i));
}

interface IconEntry {
  name: string;
  svg?: string;
  png?: string;
}

/** Adds or replaces a custom `[[icon]]`. */
export function upsertIcon(model: ProfileModel, icon: PendingIcon): void {
  const entry: IconEntry = { name: icon.name, ...(icon.svg !== undefined ? { svg: icon.svg } : { png: icon.png }) };
  const list = ((model['icon'] as IconEntry[] | undefined) ?? []).filter((i) => i.name !== icon.name);
  model['icon'] = [...list, entry];
}

/** Drops `[[icon]]` entries no rule refers to any more. */
export function pruneIcons(model: ProfileModel, rows: readonly RuleRow[]): void {
  const list = model['icon'] as IconEntry[] | undefined;
  if (!list) return;
  const used = usedIcons(rows);
  const kept = list.filter((i) => used.has(i.name));
  if (kept.length) model['icon'] = kept;
  else delete model['icon'];
}

/** Makes an icon name the engine accepts (letters, digits, '-' or '_') from a file name. */
export function iconNameFromFile(fileName: string, taken: ReadonlySet<string>): string {
  const base = fileName.replace(/\.[^.]+$/, '').replace(/[^A-Za-z0-9_-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 40) || 'icon';
  let name = base;
  for (let i = 2; taken.has(name); i++) name = `${base}-${i}`;
  return name;
}

/** Early, friendly checks before the engine validates for real. */
export function ruleProblems(rule: MarkerRuleX): string[] {
  const out: string[] = [];
  if (!rule.kind) out.push($localize`:@@markers.rules.needsId:The marker needs an id.`);
  if (rule.color && !/^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(rule.color)) out.push($localize`:@@markers.rules.colourFormat:Colour must be #RGB or #RRGGBB.`);
  if (rule.badge !== undefined && ([...rule.badge].length < 1 || [...rule.badge].length > 3)) out.push($localize`:@@markers.rules.badgeLength:A badge is 1 to 3 characters.`);
  if (!rule.when?.length) out.push($localize`:@@markers.rules.needsCondition:Add at least one condition: a marker without any would mark every event.`);
  return out;
}

/** Serialisable copy of a rule without empty optional keys. */
export function cleanRule(rule: MarkerRuleX): MarkerRuleX {
  const out: MarkerRuleX = { ...rule };
  for (const key of ['label', 'description', 'icon', 'badge', 'shape', 'color'] as const) {
    if (out[key] === undefined || out[key] === '') delete out[key];
  }
  if (!out.lifecycle) delete out.lifecycle;
  return out;
}
