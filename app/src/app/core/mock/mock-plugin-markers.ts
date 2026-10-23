/**
 * The effective marker set of the browser demo, mimicking crates/debrief-core/src/plugin/markers.rs: the
 * profile's own rules, then the marker kinds a (mock) plugin contributes, with `[[marker_override]]`
 * switching kinds off or restyling them. Not the reference for behaviour.
 */
import { MarkerKind, MarkerRole, MarkerShape, RuleCondition } from '../api.types';
import { Condition, MarkerOverride, MarkerRule, ProfileModel } from '../profile.model';

/** A marker kind contributed by a plugin (namespaced `<owner>/<kind>`). */
export interface MockPluginKind {
  owner: string;
  /** Full name, `<owner>/<kind>`. */
  kind: string;
  label: string;
  role: MarkerRole | null;
  color: string | null;
  badge: string | null;
  shape: MarkerShape | null;
  description: string | null;
  when: Condition[];
}

export const MOCK_WINDOWS_PLUGIN = 'net.decryptology.windows';

/** Kinds of the mock Windows plugin (a subset of crates/debrief-plugin-windows/src/markers). */
export const MOCK_PLUGIN_KINDS: readonly MockPluginKind[] = [
  {
    owner: MOCK_WINDOWS_PLUGIN,
    kind: `${MOCK_WINDOWS_PLUGIN}/app-crash`,
    label: 'Application crash',
    role: 'crash',
    color: '#E5484D',
    badge: 'AC',
    shape: 'hexagon',
    description: 'A process ended with an unhandled error.',
    when: [{ field: '@level', op: 'eq', value: 'Fatal' }],
  },
  {
    owner: MOCK_WINDOWS_PLUGIN,
    kind: `${MOCK_WINDOWS_PLUGIN}/service-stop`,
    label: 'Service stopped',
    role: 'stop',
    color: '#A7B0C2',
    badge: 'S',
    shape: 'square',
    description: 'A service or host process reported that it stopped.',
    when: [{ field: '@message', op: 'matches', value: '(?i)application is shutting down' }],
  },
];

/** One entry of the effective marker set. */
export interface EffectiveMarker {
  info: MarkerKind;
  when: Condition[];
}

const str = (v: unknown): string | null => (typeof v === 'string' ? v : null);

function fromRule(m: MarkerRule): EffectiveMarker {
  return {
    info: {
      kind: m.kind,
      owner: 'profile',
      role: null,
      label: m.label ?? m.kind,
      lifecycle: !!m.lifecycle,
      color: m.color ?? null,
      icon: str(m['icon']),
      badge: str(m['badge']),
      shape: str(m['shape']) as MarkerShape | null,
      description: str(m['description']),
      when: [],
    },
    when: m.when ?? [],
  };
}

function fromPlugin(k: MockPluginKind): EffectiveMarker {
  const { owner, kind, label, role, color, badge, shape, description, when } = k;
  const rule: RuleCondition[] = when.map((c) => ({ field: c.field, op: c.op, value: c.value ?? null }));
  return {
    info: { kind, owner, role, label, lifecycle: role !== null, color, icon: null, badge, shape, description, when: rule },
    when,
  };
}

function restyle(m: EffectiveMarker, o: MarkerOverride): EffectiveMarker {
  const i = m.info;
  return {
    ...m,
    info: {
      ...i,
      label: o.label ?? i.label,
      color: o.color ?? i.color,
      icon: o.icon ?? i.icon,
      badge: o.badge ?? i.badge,
      shape: (o.shape as MarkerShape | undefined) ?? i.shape,
      description: o.description ?? i.description,
    },
  };
}

/** Profile rules first, then plugin kinds, with overrides applied (overrides of unknown kinds are kept and ignored). */
export function effectiveMarkers(profile: ProfileModel, plugins: readonly MockPluginKind[] = MOCK_PLUGIN_KINDS): EffectiveMarker[] {
  let out = [...(profile.marker ?? []).map(fromRule), ...plugins.map(fromPlugin)];
  for (const o of profile.marker_override ?? []) {
    if (o.enabled === false) out = out.filter((m) => m.info.kind !== o.kind);
    else out = out.map((m) => (m.info.kind === o.kind ? restyle(m, o) : m));
  }
  return out;
}
