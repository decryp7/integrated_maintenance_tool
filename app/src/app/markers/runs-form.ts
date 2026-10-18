/**
 * The Runs editor's form model (UI state only): presets, turning the form into the engine's `SpanDraft` and back, and
 * writing what the engine planned into the profile model. Validation and the profile changes themselves are the
 * engine's (`preview_spans`); this only moves values between the form and the wire.
 */
import { SpanCfgToml, SpanDraft, SpanPlan, SpanTrigger } from '../core/api.types';
import { MarkerRule, ProfileModel } from '../core/profile.model';

/** What starts or stops a run in the form: a marker kind, or text the message matches. */
export interface TriggerForm {
  mode: 'marker' | 'pattern';
  /** Marker kind (mode "marker"). */
  marker: string;
  /** Regex over the message (mode "pattern"). */
  pattern: string;
}

export interface RunForm {
  label: string;
  /** The rule's id; derived from the label for new rules. */
  kind: string;
  /** Set when editing an existing rule (its id stays). */
  editing: string | null;
  start: TriggerForm;
  stop: TriggerForm;
  /** "" = each timeline lane, else a field name. */
  field: string;
  maxDuration: string;
  restartOnStart: boolean;
  color: string;
}

export type PresetId = 'service' | 'process' | 'custom';

export interface Preset {
  id: PresetId;
  label: string;
  hint: string;
  apply: (f: RunForm) => RunForm;
}

export const PROCESS_START_PATTERN = '\\b(process|application|service|worker)\\b.{0,40}\\b(started|starting|launched)\\b';
export const PROCESS_STOP_PATTERN = '\\b(process|application|service|worker)\\b.{0,40}\\b(exited|exiting|stopped|stopping|terminated|shut ?down|shutting down)\\b';

export function emptyForm(): RunForm {
  return {
    label: '',
    kind: '',
    editing: null,
    start: { mode: 'marker', marker: 'start', pattern: '' },
    stop: { mode: 'marker', marker: 'stop', pattern: '' },
    field: '',
    maxDuration: '',
    restartOnStart: true,
    color: '',
  };
}

/** The presets offered at the top of the form. */
export function presets(): Preset[] {
  return [
    {
      id: 'service',
      label: $localize`:Runs editor preset|@@runs.preset.service:Service started / stopped`,
      hint: $localize`:@@runs.preset.serviceHint:Uses the Start and Stop markers, on each timeline lane.`,
      apply: (f) => withLabel({ ...f, start: { mode: 'marker', marker: 'start', pattern: '' }, stop: { mode: 'marker', marker: 'stop', pattern: '' }, field: '' }, $localize`:@@runs.preset.serviceLabel:Service running`),
    },
    {
      id: 'process',
      label: $localize`:Runs editor preset|@@runs.preset.process:Process start / exit by process id`,
      hint: $localize`:@@runs.preset.processHint:Matches "process started" … "process exited" lines and pairs them per ProcessId, for logs many processes share.`,
      apply: (f) =>
        withLabel(
          { ...f, start: { mode: 'pattern', marker: '', pattern: PROCESS_START_PATTERN }, stop: { mode: 'pattern', marker: '', pattern: PROCESS_STOP_PATTERN }, field: 'ProcessId' },
          $localize`:@@runs.preset.processLabel:Process`,
        ),
    },
    {
      id: 'custom',
      label: $localize`:Runs editor preset|@@runs.preset.custom:Custom`,
      hint: $localize`:@@runs.preset.customHint:Choose the markers or the text that start and stop a run.`,
      apply: (f) => f,
    },
  ];
}

function withLabel(f: RunForm, label: string): RunForm {
  return f.editing ? f : { ...f, label, kind: slug(label) };
}

/** A rule id from a label: lower case, letters, digits and dashes ("Service running" → "service-running"). */
export function slug(label: string): string {
  const s = label
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 64);
  return s || 'run';
}

function trigger(t: TriggerForm): SpanTrigger {
  return t.mode === 'pattern' ? { type: 'pattern', pattern: t.pattern } : { type: 'markers', kinds: t.marker ? [t.marker] : [] };
}

/** The form as the engine's candidate rule. */
export function toDraft(f: RunForm): SpanDraft {
  return {
    kind: f.editing ?? (f.kind || slug(f.label)),
    label: f.label.trim(),
    start: trigger(f.start),
    stop: trigger(f.stop),
    per: f.field ? `field:${f.field}` : 'instance',
    maxDuration: f.maxDuration.trim() || null,
    restartOnStart: f.restartOnStart,
    color: f.color.trim() || null,
  };
}

const first = (v: string | string[]): string => (Array.isArray(v) ? (v[0] ?? '') : v);

/** An existing rule in the form, to edit it. Rules naming several markers keep only the first in the form. */
export function fromRule(r: SpanCfgToml): RunForm {
  const per = r.per ?? 'instance';
  return {
    label: r.label ?? r.kind,
    kind: r.kind,
    editing: r.kind,
    start: { mode: 'marker', marker: first(r.start), pattern: '' },
    stop: { mode: 'marker', marker: first(r.stop), pattern: '' },
    field: per.startsWith('field:') ? per.slice(6).trim() : '',
    maxDuration: r.max_duration ?? '',
    restartOnStart: r.restart_on_start !== false,
    color: r.color ?? '',
  };
}

/** Writes what the engine planned: the whole `[[span]]` list, and each needed marker rule (replacing one of its kind). */
export function applyPlan(model: ProfileModel, plan: SpanPlan): void {
  model.span = plan.spans.map((s) => ({ ...s }));
  const markers: MarkerRule[] = [...(model.marker ?? [])];
  for (const m of plan.markers) {
    const rule = { ...m } as MarkerRule;
    const at = markers.findIndex((x) => x.kind === m.kind);
    if (at >= 0) markers[at] = rule;
    else markers.push(rule);
  }
  model.marker = markers;
}

/** The rules with one switched on or off (`rules` is the profile's list as the engine reported it). */
export function withEnabled(rules: readonly SpanCfgToml[], kind: string, enabled: boolean): SpanCfgToml[] {
  return rules.map((r) => (r.kind !== kind ? { ...r } : enabled ? withoutEnabled(r) : { ...r, enabled: false }));
}

function withoutEnabled(r: SpanCfgToml): SpanCfgToml {
  const { enabled: _enabled, ...rest } = r;
  return rest;
}

/** The rules without one. */
export function without(rules: readonly SpanCfgToml[], kind: string): SpanCfgToml[] {
  return rules.filter((r) => r.kind !== kind).map((r) => ({ ...r }));
}
