/**
 * Pure operations on `[[marker_override]]`: switching a plugin (or built-in) marker kind off and restyling it.
 * The rules of such kinds belong to their plugin and are read-only; only these overrides are written to the
 * profile (validated and saved by the engine like every other profile change).
 */
import { BUILTIN_OWNER, MarkerKind, MarkerShape, PROFILE_OWNER, RuleCondition } from '../core/api.types';
import { Condition, ConditionOp, MarkerOverride, ProfileModel } from '../core/profile.model';

/** The style keys an override can change. */
export type OverrideStyle = Partial<Pick<MarkerOverride, 'label' | 'color' | 'icon' | 'badge' | 'shape' | 'description'>>;

const STYLE_KEYS = ['label', 'color', 'icon', 'badge', 'shape', 'description'] as const;

/** True for kinds a plugin contributes (not the profile's own rules nor the built-in defaults). */
export function isPluginOwned(owner: string): boolean {
  return owner !== PROFILE_OWNER && owner !== BUILTIN_OWNER;
}

/** A plugin marker kind as the manager lists it. */
export interface PluginKindRow {
  kind: string;
  owner: string;
  label: string;
  enabled: boolean;
  /** As the engine reports it (null for a kind switched off: the engine no longer lists it). */
  info: MarkerKind | null;
  /** The plugin's default rule, read-only. */
  when: Condition[];
  /** Style keys the profile overrides. */
  restyled: (keyof OverrideStyle)[];
}

export function readOverrides(model: ProfileModel): MarkerOverride[] {
  return Array.isArray(model.marker_override) ? model.marker_override : [];
}

/** Owner of a namespaced kind (`<plugin-id>/<kind>`); empty when it has none. */
export function ownerOfKind(kind: string): string {
  const i = kind.lastIndexOf('/');
  return i > 0 ? kind.slice(0, i) : '';
}

/** A rule condition from the engine as a profile condition (lists are shown comma separated). */
export function toCondition(c: RuleCondition): Condition {
  const value = c.value === null ? undefined : Array.isArray(c.value) ? c.value.join(', ') : String(c.value);
  return value === undefined ? { field: c.field, op: c.op as ConditionOp } : { field: c.field, op: c.op as ConditionOp, value };
}

/**
 * The plugin kinds in the engine's order, then those the profile switched off (which the engine no longer
 * lists), so they can be switched on again.
 */
export function pluginRows(kinds: readonly MarkerKind[], model: ProfileModel): PluginKindRow[] {
  const overrides = readOverrides(model);
  const restyled = (kind: string) => {
    const o = overrides.find((x) => x.kind === kind);
    return o ? STYLE_KEYS.filter((k) => o[k] !== undefined) : [];
  };
  const active: PluginKindRow[] = kinds
    .filter((k) => isPluginOwned(k.owner))
    .map((k) => ({ kind: k.kind, owner: k.owner, label: k.label, enabled: true, info: k, when: k.when.map(toCondition), restyled: restyled(k.kind) }));
  const listed = new Set(active.map((r) => r.kind));
  const off: PluginKindRow[] = overrides
    .filter((o) => o.enabled === false && !listed.has(o.kind) && ownerOfKind(o.kind) !== '')
    .map((o) => ({ kind: o.kind, owner: ownerOfKind(o.kind), label: o.label ?? o.kind, enabled: false, info: null, when: [], restyled: restyled(o.kind) }));
  return [...active, ...off];
}

/**
 * Sets (or with `undefined`/empty values, clears) override keys for `kind`. `enabled: true` is the default and is
 * not written; an override left with only its kind is removed, and so is an empty `[[marker_override]]` list.
 */
export function setOverride(model: ProfileModel, kind: string, patch: OverrideStyle & { enabled?: boolean }): void {
  const list = readOverrides(model).map((o) => ({ ...o }));
  let entry = list.find((o) => o.kind === kind);
  if (!entry) {
    entry = { kind };
    list.push(entry);
  }
  const target = entry as unknown as Record<string, unknown>;
  for (const [key, value] of Object.entries(patch)) {
    if (value === undefined || value === '' || (key === 'enabled' && value === true)) delete target[key];
    else target[key] = value;
  }
  const kept = list.filter((o) => Object.keys(o).some((k) => k !== 'kind'));
  if (kept.length) model.marker_override = kept;
  else delete model.marker_override;
}

/** Removes every style override of `kind` (keeps it switched off if it was). */
export function resetStyle(model: ProfileModel, kind: string): void {
  setOverride(model, kind, Object.fromEntries(STYLE_KEYS.map((k) => [k, undefined])) as OverrideStyle);
}

/** Early checks of a restyle; the engine validates for real. */
export function styleProblems(style: OverrideStyle): string[] {
  const out: string[] = [];
  if (style.color && !/^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(style.color)) out.push('Colour must be #RGB or #RRGGBB.');
  if (style.badge !== undefined && style.badge !== '' && [...style.badge].length > 3) out.push('A badge is 1 to 3 characters.');
  if (style.shape && !(['circle', 'square', 'diamond', 'hexagon'] as MarkerShape[]).includes(style.shape as MarkerShape)) out.push('Unknown badge shape.');
  return out;
}
