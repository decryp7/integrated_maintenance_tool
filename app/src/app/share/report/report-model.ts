import { Report, ReportBlock, ReportSection } from '../../core/api.types';
import { newId } from '../../projects/bookmarks/ids';

/**
 * Pure edits of the report document (the engine's `Report`, crates/debrief-core/src/report). Every function
 * returns a new report; the engine re-validates the document whenever it is saved or exported.
 */

export const REPORT_VERSION = 1;

/** A block before it gets its id. */
export type NewBlock = ReportBlock extends infer B ? (B extends ReportBlock ? Omit<B, 'id'> : never) : never;

export function newReport(title = $localize`:@@share.reportModel.defaultTitle:Incident report`, now = Date.now()): Report {
  return {
    version: REPORT_VERSION,
    id: newId('rep'),
    title,
    summary: '',
    createdAt: now,
    updatedAt: now,
    sections: [newSection($localize`:@@share.reportModel.timelineSection:Timeline`)],
  };
}

export function newSection(title = $localize`:@@share.reportModel.defaultSection:Section`): ReportSection {
  return { id: newId('sec'), title, blocks: [] };
}

export function withId(block: NewBlock): ReportBlock {
  return { ...block, id: newId('blk') } as ReportBlock;
}

const touch = (r: Report, now: number): Report => ({ ...r, updatedAt: now });

/** Appends a block to a section (the last one when `sectionId` is absent or unknown; a new one when there is none). */
export function addBlock(r: Report, block: ReportBlock, sectionId?: string | null, now = Date.now()): Report {
  const sections = r.sections.length ? r.sections : [newSection($localize`:@@share.reportModel.timelineSection:Timeline`)];
  const target = sections.find((s) => s.id === sectionId) ?? sections[sections.length - 1];
  return touch({ ...r, sections: sections.map((s) => (s === target ? { ...s, blocks: [...s.blocks, block] } : s)) }, now);
}

export function updateBlock(r: Report, id: string, patch: Partial<ReportBlock>, now = Date.now()): Report {
  return touch({ ...r, sections: r.sections.map((s) => ({ ...s, blocks: s.blocks.map((b) => (b.id === id ? ({ ...b, ...patch, id, type: b.type } as ReportBlock) : b)) })) }, now);
}

export function removeBlock(r: Report, id: string, now = Date.now()): Report {
  return touch({ ...r, sections: r.sections.map((s) => ({ ...s, blocks: s.blocks.filter((b) => b.id !== id) })) }, now);
}

/** Where a block is: section index and position. */
export function locate(r: Report, id: string): { section: number; index: number } | null {
  for (let section = 0; section < r.sections.length; section++) {
    const index = r.sections[section]?.blocks.findIndex((b) => b.id === id) ?? -1;
    if (index >= 0) return { section, index };
  }
  return null;
}

/**
 * Moves a block up (`delta < 0`) or down; at the edge of its section it moves into the previous or next
 * section. Unknown ids and moves past the first/last block change nothing.
 */
export function moveBlock(r: Report, id: string, delta: -1 | 1, now = Date.now()): Report {
  const at = locate(r, id);
  if (!at) return r;
  const sections = r.sections.map((s) => ({ ...s, blocks: [...s.blocks] }));
  const blocks = sections[at.section]?.blocks;
  const [block] = blocks?.splice(at.index, 1) ?? [];
  if (!blocks || !block) return r;
  const to = at.index + delta;
  if (to >= 0 && to <= blocks.length && !(delta > 0 && at.index === blocks.length)) {
    blocks.splice(to, 0, block);
  } else {
    const next = at.section + delta;
    const target = sections[next];
    if (!target) return r;
    if (delta < 0) target.blocks.push(block);
    else target.blocks.unshift(block);
  }
  return touch({ ...r, sections }, now);
}

export function moveSection(r: Report, id: string, delta: -1 | 1, now = Date.now()): Report {
  const i = r.sections.findIndex((s) => s.id === id);
  const j = i + delta;
  if (i < 0 || j < 0 || j >= r.sections.length) return r;
  const sections = [...r.sections];
  const a = sections[i];
  const b = sections[j];
  if (!a || !b) return r;
  sections[i] = b;
  sections[j] = a;
  return touch({ ...r, sections }, now);
}

/** Keeps only well-formed reports of untrusted saved state (the engine checks the rest). */
export function parseReports(value: unknown): Report[] {
  if (!Array.isArray(value)) return [];
  const out: Report[] = [];
  for (const raw of value as Partial<Report>[]) {
    if (!raw || typeof raw !== 'object' || typeof raw.id !== 'string' || !raw.id) continue;
    const sections = Array.isArray(raw.sections) ? raw.sections : [];
    out.push({
      version: typeof raw.version === 'number' ? raw.version : REPORT_VERSION,
      id: raw.id,
      title: typeof raw.title === 'string' ? raw.title : '',
      summary: typeof raw.summary === 'string' ? raw.summary : '',
      createdAt: typeof raw.createdAt === 'number' ? raw.createdAt : 0,
      updatedAt: typeof raw.updatedAt === 'number' ? raw.updatedAt : 0,
      sections: sections
        .filter((s): s is ReportSection => !!s && typeof s === 'object' && typeof s.id === 'string')
        .map((s) => ({ id: s.id, title: typeof s.title === 'string' ? s.title : '', blocks: Array.isArray(s.blocks) ? s.blocks.filter((b) => !!b && typeof b === 'object' && typeof b.id === 'string' && typeof b.type === 'string') : [] })),
    });
  }
  return out;
}

/** Every block of a report, in order. */
export function blocksOf(r: Report): ReportBlock[] {
  return r.sections.flatMap((s) => s.blocks);
}
