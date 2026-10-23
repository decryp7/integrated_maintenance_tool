import { Query } from './api.types';
import { EMPTY_QUERY } from './workspace.store';

const QUERY_KEYS = Object.keys(EMPTY_QUERY) as (keyof Query)[];

/** Keeps only known query keys of untrusted saved state, on top of the empty query. */
export function sanitizeQuery(value: unknown): Query {
  const given = value && typeof value === 'object' ? (value as Record<string, unknown>) : {};
  const query: Record<string, unknown> = { ...EMPTY_QUERY };
  for (const k of QUERY_KEYS) if (k in given) query[k] = given[k];
  return query as unknown as Query;
}
