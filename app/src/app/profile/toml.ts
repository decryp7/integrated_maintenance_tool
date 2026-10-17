import { parse, stringify } from 'smol-toml';
import { ProfileModel } from '../core/profile.model';

/** Removes undefined values (TOML has no null) so stringify never fails. */
export function prune(value: unknown): unknown {
  if (Array.isArray(value)) return value.filter((v) => v !== undefined).map(prune);
  if (value && typeof value === 'object') {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value)) if (v !== undefined) out[k] = prune(v);
    return out;
  }
  return value;
}

export function parseProfile(toml: string): ProfileModel {
  return parse(toml) as ProfileModel;
}

export function stringifyProfile(model: ProfileModel): string {
  return stringify(prune(model) as Record<string, unknown>);
}
