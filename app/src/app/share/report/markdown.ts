/**
 * The report's Markdown subset (mirrors crates/imt-core/src/report/markdown.rs) parsed into plain data
 * for an on-screen preview. Templates render the data as text nodes: nothing is ever parsed as HTML.
 */

export type MdInline = { style: 'text' | 'strong' | 'em' | 'code'; text: string } | { style: 'link'; text: string; url: string };

export type MdBlock =
  | { kind: 'heading'; level: 1 | 2 | 3; inlines: MdInline[] }
  | { kind: 'paragraph' | 'quote'; inlines: MdInline[] }
  | { kind: 'list'; ordered: boolean; items: MdInline[][] }
  | { kind: 'code'; text: string }
  | { kind: 'rule' };

function delimited(s: string, open: string, close: string): [string, number] | null {
  if (!s.startsWith(open)) return null;
  const body = s.slice(open.length);
  const end = body.indexOf(close);
  if (end <= 0) return null;
  const inner = body.slice(0, end);
  if (inner.trim() !== inner) return null;
  return [inner, open.length + end + close.length];
}

function styled(s: string): [MdInline, number] | null {
  const strong = delimited(s, '**', '**');
  if (strong) return [{ style: 'strong', text: strong[0] }, strong[1]];
  const code = delimited(s, '`', '`');
  if (code) return [{ style: 'code', text: code[0] }, code[1]];
  const em = delimited(s, '*', '*') ?? delimited(s, '_', '_');
  if (em) return [{ style: 'em', text: em[0] }, em[1]];
  const label = delimited(s, '[', ']');
  if (!label) return null;
  const after = s.slice(label[1]);
  if (!after.startsWith('(')) return null;
  const end = after.indexOf(')');
  if (end < 0) return null;
  const url = after.slice(1, end).trim();
  const safe = /^(https?:\/\/|mailto:)/i.test(url);
  return [safe ? { style: 'link', text: label[0], url } : { style: 'text', text: label[0] }, label[1] + end + 1];
}

/** Inline styles of one line. */
export function parseInline(text: string): MdInline[] {
  const out: MdInline[] = [];
  let plain = '';
  let rest = text;
  while (rest.length) {
    const hit = styled(rest);
    if (hit) {
      if (plain) out.push({ style: 'text', text: plain });
      plain = '';
      out.push(hit[0]);
      rest = rest.slice(hit[1]);
    } else {
      const c = String.fromCodePoint(rest.codePointAt(0)!);
      plain += c;
      rest = rest.slice(c.length);
    }
  }
  if (plain) out.push({ style: 'text', text: plain });
  return out;
}

/** Parses the supported subset; anything else is plain text. */
export function parseMarkdown(text: string): MdBlock[] {
  const out: MdBlock[] = [];
  let para: string[] = [];
  let list: { ordered: boolean; items: MdInline[][] } | null = null;
  let code: string[] | null = null;
  const flushPara = () => {
    if (para.length) out.push({ kind: 'paragraph', inlines: parseInline(para.join(' ')) });
    para = [];
  };
  const flushList = () => {
    if (list) out.push({ kind: 'list', ...list });
    list = null;
  };
  for (const line of text.split(/\r?\n/)) {
    if (code) {
      if (line.trimStart().startsWith('```')) {
        out.push({ kind: 'code', text: code.join('\n') });
        code = null;
      } else code.push(line);
      continue;
    }
    const t = line.trim();
    if (t.startsWith('```')) {
      flushPara();
      flushList();
      code = [];
      continue;
    }
    if (!t) {
      flushPara();
      flushList();
      continue;
    }
    const item = /^[-*] (.*)$/.exec(t) ?? null;
    const numbered = item ? null : /^\d+\. (.*)$/.exec(t);
    if (item || numbered) {
      flushPara();
      const ordered = !!numbered;
      const body = parseInline((item ?? numbered)![1].trim());
      if (list && list.ordered === ordered) list.items.push(body);
      else {
        flushList();
        list = { ordered, items: [body] };
      }
      continue;
    }
    flushList();
    const heading = /^(#{1,3}) (.*)$/.exec(t);
    if (t === '---' || t === '***') {
      flushPara();
      out.push({ kind: 'rule' });
    } else if (heading) {
      flushPara();
      out.push({ kind: 'heading', level: heading[1].length as 1 | 2 | 3, inlines: parseInline(heading[2].trim()) });
    } else if (t.startsWith('>')) {
      flushPara();
      out.push({ kind: 'quote', inlines: parseInline(t.slice(1).trim()) });
    } else para.push(t);
  }
  if (code) out.push({ kind: 'code', text: (code as string[]).join('\n') });
  flushPara();
  flushList();
  return out;
}
