import { TestBed } from '@angular/core/testing';
import { Report, ReportBlock } from '../../core/api.types';
import { parseInline, parseMarkdown } from './markdown';
import { addBlock, blocksOf, locate, moveBlock, moveSection, newReport, newSection, parseReports, removeBlock, updateBlock, withId } from './report-model';
import { reportEventIds, reportText } from './report-text';
import { ReportStore } from './report.store';

const text = (markdown: string): ReportBlock => withId({ type: 'text', markdown });

function sample(): Report {
  const r = newReport('R', 1);
  const a = text('a');
  const b = text('b');
  const c = text('c');
  return { ...r, sections: [{ ...r.sections[0], blocks: [a, b] }, { id: 's2', title: 'Two', blocks: [c] }] };
}

const order = (r: Report) => blocksOf(r).map((b) => (b as { markdown: string }).markdown);

describe('report model', () => {
  it('creates reports, sections and blocks with ids', () => {
    const r = newReport(undefined, 5);
    expect(r).toMatchObject({ version: 1, title: 'Incident report', createdAt: 5, updatedAt: 5, summary: '' });
    expect(r.sections[0].title).toBe('Timeline');
    expect(newSection().title).toBe('Section');
    expect(withId({ type: 'text', markdown: 'x' }).id).toMatch(/^blk/);
  });

  it('adds to a section, the last one by default, or a new one', () => {
    const r = sample();
    expect(order(addBlock(r, text('d'), r.sections[0].id, 9))).toEqual(['a', 'b', 'd', 'c']);
    expect(order(addBlock(r, text('d'), 'nope'))).toEqual(['a', 'b', 'c', 'd']);
    const empty = { ...r, sections: [] };
    const added = addBlock(empty, text('x'), null, 7);
    expect(added.sections).toHaveLength(1);
    expect(added.updatedAt).toBe(7);
  });

  it('updates and removes blocks but never their id or type', () => {
    const r = sample();
    const id = r.sections[0].blocks[0].id;
    const u = updateBlock(r, id, { markdown: 'A', id: 'other', type: 'events' } as never);
    expect(u.sections[0].blocks[0]).toEqual({ id, type: 'text', markdown: 'A' });
    expect(order(removeBlock(r, id))).toEqual(['b', 'c']);
    expect(locate(r, 'missing')).toBeNull();
    expect(locate(r, r.sections[1].blocks[0].id)).toEqual({ section: 1, index: 0 });
  });

  it('moves blocks within and across sections, and stops at the ends', () => {
    const r = sample();
    const [a, b] = r.sections[0].blocks;
    const c = r.sections[1].blocks[0];
    expect(order(moveBlock(r, a.id, 1))).toEqual(['b', 'a', 'c']);
    expect(order(moveBlock(r, b.id, -1))).toEqual(['b', 'a', 'c']);
    const down = moveBlock(r, b.id, 1);
    expect(down.sections[1].blocks.map((x) => x.id)).toEqual([b.id, c.id]);
    const up = moveBlock(r, c.id, -1);
    expect(up.sections[0].blocks.map((x) => x.id)).toEqual([a.id, b.id, c.id]);
    expect(moveBlock(r, a.id, -1)).toBe(r);
    expect(moveBlock(r, c.id, 1)).toBe(r);
    expect(moveBlock(r, 'missing', 1)).toBe(r);
  });

  it('moves sections', () => {
    const r = sample();
    expect(moveSection(r, 's2', -1).sections.map((s) => s.id)).toEqual(['s2', r.sections[0].id]);
    expect(moveSection(r, 's2', 1)).toBe(r);
    expect(moveSection(r, 'x', 1)).toBe(r);
  });

  it('keeps only well-formed reports from saved state', () => {
    expect(parseReports(null)).toEqual([]);
    const parsed = parseReports([null, { id: '' }, { id: 'r', sections: [null, { id: 's', blocks: [{ id: 'b', type: 'text', markdown: 'x' }, { id: 1 }, null] }, { title: 'no id' }] }, { id: 'q', version: 2, title: 'T', summary: 'S', createdAt: 1, updatedAt: 2, sections: 'nope' }]);
    expect(parsed).toHaveLength(2);
    expect(parsed[0]).toMatchObject({ version: 1, title: '', summary: '', createdAt: 0, updatedAt: 0 });
    expect(parsed[0].sections).toEqual([{ id: 's', title: '', blocks: [{ id: 'b', type: 'text', markdown: 'x' }] }]);
    expect(parsed[1]).toMatchObject({ version: 2, title: 'T', summary: 'S', sections: [] });
    expect(parseReports([{ id: 'x', sections: [{ id: 's', title: 'T', blocks: 'x' }] }])[0].sections[0].blocks).toEqual([]);
  });

  it('collects the free text and event ids of a report', () => {
    const r = newReport('Title', 1);
    r.summary = 'Summary';
    r.sections[0].blocks = [
      withId({ type: 'text', markdown: 'md' }),
      withId({ type: 'events', caption: 'cap', ids: [3, 1, 3] }),
      withId({ type: 'chart', title: 'chart t', caption: '' }),
      withId({ type: 'chart' }),
      withId({ type: 'timeline', svg: null }),
      withId({ type: 'table', columns: ['c1'], rows: [['v', null, undefined, 4]] }),
      withId({ type: 'finding', severity: 'warning', title: 'ft', detail: 'fd' }),
      withId({ type: 'annotations', items: [{ from: 1, label: 'al' }] }),
      withId({ type: 'events', ids: [2] }),
    ];
    expect(reportText(r).split('\n')).toEqual(['Title', 'Summary', 'Timeline', 'md', 'cap', 'chart t', 'c1', 'v   4', 'ft', 'fd', 'al']);
    expect(reportEventIds(r)).toEqual([3, 1, 2]);
  });
});

describe('report markdown', () => {
  it('parses blocks like the engine', () => {
    const b = parseMarkdown('# Title\nfirst\nsecond\n\n- a\n- **b**\n1. one\n2. two\n> q\n---\n```\ncode <x>\n```\n## Sub\n#### not\ntext\n```\nopen');
    expect(b.map((x) => x.kind)).toEqual(['heading', 'paragraph', 'list', 'list', 'quote', 'rule', 'code', 'heading', 'paragraph', 'code']);
    expect(b[1]).toEqual({ kind: 'paragraph', inlines: [{ style: 'text', text: 'first second' }] });
    expect(b[3]).toMatchObject({ kind: 'list', ordered: true });
    expect(b[6]).toEqual({ kind: 'code', text: 'code <x>' });
    expect(b[8]).toEqual({ kind: 'paragraph', inlines: [{ style: 'text', text: '#### not text' }] });
    expect(parseMarkdown('')).toEqual([]);
    expect(parseMarkdown('- x\n\ny').length).toBe(2);
    expect(parseMarkdown('1. a\n- b').length).toBe(2);
    expect(parseMarkdown('p\n- x\ntext').length).toBe(3);
    expect(parseMarkdown('***')).toEqual([{ kind: 'rule' }]);
  });

  it('parses inline styles and keeps only safe links', () => {
    const i = parseInline('a **b** *c* _d_ `e` [f](https://x.io) [g](javascript:alert(1)) [h](mailto:a@b) * x * [y] [z](w');
    expect(i.slice(0, 2)).toEqual([{ style: 'text', text: 'a ' }, { style: 'strong', text: 'b' }]);
    expect(i[3]).toEqual({ style: 'em', text: 'c' });
    expect(i[5]).toEqual({ style: 'em', text: 'd' });
    expect(i[7]).toEqual({ style: 'code', text: 'e' });
    expect(i[9]).toEqual({ style: 'link', text: 'f', url: 'https://x.io' });
    expect(i[11]).toEqual({ style: 'text', text: 'g' });
    expect(i[13]).toEqual({ style: 'link', text: 'h', url: 'mailto:a@b' });
    expect(i.slice(14).map((x) => x.text).join('')).toBe(' * x * [y] [z](w');
    expect(parseInline('ü 😀 *é*')).toEqual([{ style: 'text', text: 'ü 😀 ' }, { style: 'em', text: 'é' }]);
  });
});

describe('ReportStore', () => {
  beforeEach(() => TestBed.configureTestingModule({}));

  it('creates a report on first add and edits the active one', () => {
    const s = TestBed.inject(ReportStore);
    expect(s.active()).toBeNull();
    const b = s.add({ type: 'text', markdown: 'x' });
    const r = s.active()!;
    expect(blocksOf(r)).toEqual([b]);
    s.update(r.id, { title: 'New title' });
    s.update('missing', { title: 'x' });
    expect(s.active()!.title).toBe('New title');
    s.updateBlock(b.id, { markdown: 'y' });
    const c = s.add({ type: 'text', markdown: 'z' });
    s.moveBlock(c.id, -1);
    expect(order(s.active()!)).toEqual(['z', 'y']);
    s.removeBlock(c.id);
    expect(order(s.active()!)).toEqual(['y']);
    const sec = s.addSection('Analysis');
    s.renameSection(sec.id, 'Cause');
    s.moveSection(sec.id, -1);
    expect(s.active()!.sections.map((x) => x.title)).toEqual(['Cause', 'Timeline']);
    s.removeSection(sec.id);
    expect(s.active()!.sections).toHaveLength(1);
    expect(JSON.parse(s.json())).toHaveLength(1);
  });

  it('switches, removes and replaces reports', () => {
    const s = TestBed.inject(ReportStore);
    const a = s.create('A');
    const b = s.create('B');
    expect(s.active()!.id).toBe(b.id);
    s.setActive(a.id);
    expect(s.active()!.title).toBe('A');
    s.remove(b.id);
    expect(s.active()!.id).toBe(a.id);
    s.remove(a.id);
    expect(s.active()).toBeNull();
    s.addSection();
    expect(s.reports()).toHaveLength(1);
    s.replace([{ id: 'r1', title: 'Saved' }, 'junk']);
    expect(s.reports().map((r) => r.title)).toEqual(['Saved']);
    s.replace('nonsense');
    expect(s.reports()).toEqual([]);
    // Edits without a report change nothing.
    s.updateBlock('x', {});
    s.removeBlock('x');
    s.moveBlock('x', 1);
    s.renameSection('x', 'y');
    s.removeSection('x');
    s.moveSection('x', 1);
    expect(s.reports()).toEqual([]);
  });
});
