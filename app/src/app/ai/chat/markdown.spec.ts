import { describe, expect, it } from 'vitest';
import { Citation } from '../ai.types';
import { Block, isNotFound, MAX_RENDER_CHARS, parseInline, parseMarkdown } from './markdown';
import { at } from '../../../testing/must';

const cites: Citation[] = [
  { kind: 'record', id: 12, verified: true },
  { kind: 'record', id: 4, verified: true },
  { kind: 'record', id: 7, verified: false },
  { kind: 'record', id: 9, verified: true },
  { kind: 'query', ql: 'level >= error' },
  { kind: 'web', title: 'Docs', url: 'https://learn.example.com/t' },
  { kind: 'web', title: 'Plain', url: 'http://insecure.example.com/' },
];

describe('parseInline', () => {
  it('keeps markup in model text as plain text', () => {
    const evil = '<img src=x onerror=alert(1)> <script>alert(2)</script> [click](javascript:alert(3)) javascript:alert(4)';
    const out = parseInline(evil, cites);
    expect(out).toEqual([{ kind: 'text', text: evil }]);
  });

  it('formats code, bold and italics', () => {
    expect(parseInline('a `x = 1` **b** __c__ *d* _e_ f')).toEqual([
      { kind: 'text', text: 'a ' },
      { kind: 'code', text: 'x = 1' },
      { kind: 'text', text: ' ' },
      { kind: 'strong', text: 'b' },
      { kind: 'text', text: ' ' },
      { kind: 'strong', text: 'c' },
      { kind: 'text', text: ' ' },
      { kind: 'em', text: 'd' },
      { kind: 'text', text: ' ' },
      { kind: 'em', text: 'e' },
      { kind: 'text', text: ' f' },
    ]);
  });

  it('links only cited records, queries and https pages', () => {
    const out = parseInline('See event 12, events 4 and 7, #9, event 99 and `level >= error` or `other` at https://learn.example.com/t. Also http://insecure.example.com/ and https://uncited.example.com', cites);
    const links = out.filter((i) => i.kind !== 'text');
    expect(links).toEqual([
      { kind: 'record', text: '12', id: 12, verified: true },
      { kind: 'record', text: '4', id: 4, verified: true },
      { kind: 'record', text: '7', id: 7, verified: false },
      { kind: 'record', text: '#9', id: 9, verified: true },
      { kind: 'query', text: 'level >= error', ql: 'level >= error' },
      { kind: 'code', text: 'other' },
      { kind: 'web', text: 'https://learn.example.com/t', url: 'https://learn.example.com/t', title: 'Docs' },
    ]);
    expect(out.map((i) => i.text).join('')).toContain('event 99');
  });

  it('does not link record numbers inside a cited URL, nor anything without citations', () => {
    const c: Citation[] = [{ kind: 'record', id: 5, verified: true }, { kind: 'web', title: 'T', url: 'https://x.example/#5' }];
    expect(parseInline('https://x.example/#5', c)).toEqual([{ kind: 'web', text: 'https://x.example/#5', url: 'https://x.example/#5', title: 'T' }]);
    expect(parseInline('event 5 and #6')).toEqual([{ kind: 'text', text: 'event 5 and #6' }]);
  });
});

describe('parseMarkdown', () => {
  const kinds = (b: Block[]) => b.map((x) => x.kind);

  it('reads paragraphs, headings, lists, continuation lines and code fences', () => {
    const md = '# Title\nfirst line\nsecond line\n\n- one\n  more of one\n* two\n1. a\n2) b\n\n```\n<b>code</b>\n```\nafter';
    const b = parseMarkdown(md, cites);
    expect(kinds(b)).toEqual(['h', 'p', 'ul', 'ol', 'code', 'p']);
    expect(b[0]).toEqual({ kind: 'h', level: 1, inlines: [{ kind: 'text', text: 'Title' }] });
    expect(b[1]).toEqual({ kind: 'p', inlines: [{ kind: 'text', text: 'first line second line' }] });
    expect(b[2]).toEqual({ kind: 'ul', items: [[{ kind: 'text', text: 'one more of one' }], [{ kind: 'text', text: 'two' }]] });
    expect(b[3]).toEqual({ kind: 'ol', items: [[{ kind: 'text', text: 'a' }], [{ kind: 'text', text: 'b' }]] });
    expect(b[4]).toEqual({ kind: 'code', text: '<b>code</b>' });
  });

  it('shows an unclosed fence (still streaming) as code and switches list kinds', () => {
    expect(parseMarkdown('text\n```\npartial')).toEqual([{ kind: 'p', inlines: [{ kind: 'text', text: 'text' }] }, { kind: 'code', text: 'partial' }]);
    expect(kinds(parseMarkdown('- a\n1. b\nplain'))).toEqual(['ul', 'ol', 'p']);
    expect(parseMarkdown('\r\n\r\n')).toEqual([]);
  });

  it('cuts runaway answers', () => {
    const b = parseMarkdown('x'.repeat(MAX_RENDER_CHARS + 50));
    const t = at((b[0] as { inlines: { text: string }[] }).inlines, 0).text;
    expect(t.length).toBe(MAX_RENDER_CHARS + 1);
    expect(t.endsWith('…')).toBe(true);
  });
});

describe('isNotFound', () => {
  it('recognises "not found in the logs" answers', () => {
    expect(isNotFound('Not found in the logs: no event mentions that.')).toBe(true);
    expect(isNotFound('The error was found in the logs.')).toBe(false);
  });
});
