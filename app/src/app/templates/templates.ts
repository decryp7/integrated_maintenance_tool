import { ParsePreview, ProfileTemplate } from '../core/api.types';

/** Templates by category (categories and templates keep the engine's order). */
export function groupTemplates(templates: readonly ProfileTemplate[]): { category: string; templates: ProfileTemplate[] }[] {
  const groups = new Map<string, ProfileTemplate[]>();
  for (const t of templates) groups.set(t.category, [...(groups.get(t.category) ?? []), t]);
  return [...groups].map(([category, list]) => ({ category, templates: list }));
}

/** Share of the previewed lines a template reads as events (0 when it reads none or failed). */
export function readRatio(p: ParsePreview): number {
  if (p.error) return 0;
  const lines = p.total + p.skipped;
  return lines ? p.total / lines : 0;
}

/** The template that reads the most of the user's file; null when none reads anything. */
export function bestTemplate(ratios: ReadonlyMap<string, number>): string | null {
  let best: string | null = null;
  let score = 0;
  for (const [id, r] of ratios) {
    if (r > score) {
      best = id;
      score = r;
    }
  }
  return best;
}

/** "reads 95%" badge text. */
export function ratioLabel(r: number): string {
  return r <= 0 ? 'reads nothing' : `reads ${Math.round(r * 100)}%`;
}
