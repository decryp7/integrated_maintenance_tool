import { Report } from '../../core/api.types';

/** Every free text of a report (what redaction also applies to), one per line, for a redaction preview. */
export function reportText(r: Report): string {
  const out: string[] = [r.title, r.summary];
  for (const s of r.sections) {
    out.push(s.title);
    for (const b of s.blocks) {
      if ('caption' in b && b.caption) out.push(b.caption);
      switch (b.type) {
        case 'text':
          out.push(b.markdown);
          break;
        case 'chart':
          if (b.title) out.push(b.title);
          break;
        case 'table':
          out.push(b.columns.join(' '), ...b.rows.map((row) => row.map((c) => (c === null || c === undefined ? '' : String(c))).join(' ')));
          break;
        case 'finding':
          out.push(b.title, b.detail);
          break;
        case 'annotations':
          out.push(...b.items.map((a) => a.label));
          break;
      }
    }
  }
  return out.filter((t) => t).join('\n');
}

/** Ids of every event a report references. */
export function reportEventIds(r: Report): number[] {
  return [...new Set(r.sections.flatMap((s) => s.blocks.flatMap((b) => (b.type === 'events' ? b.ids : []))))];
}
