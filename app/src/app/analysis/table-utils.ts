/** Presentation of an engine table: display text, sorting and CSV export. */

export function cellText(v: unknown): string {
  if (v === null || v === undefined) return '';
  if (typeof v === 'object') return JSON.stringify(v);
  return String(v);
}

export type SortDir = 'asc' | 'desc';

const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);

/** Stable sort of `rows` by column; numbers by value, everything else by text, empty cells last. */
export function sortRows(rows: readonly unknown[][], col: number, dir: SortDir): unknown[][] {
  const sign = dir === 'asc' ? 1 : -1;
  return rows
    .map((row, i) => ({ row, i }))
    .sort((a, b) => {
      const x = a.row[col];
      const y = b.row[col];
      const xe = x === null || x === undefined || x === '';
      const ye = y === null || y === undefined || y === '';
      if (xe || ye) return xe === ye ? a.i - b.i : xe ? 1 : -1;
      const c = isNum(x) && isNum(y) ? x - y : cellText(x).localeCompare(cellText(y), undefined, { numeric: true });
      return c ? c * sign : a.i - b.i;
    })
    .map((e) => e.row);
}

function csvField(v: unknown): string {
  let s = cellText(v);
  // Spreadsheet formula injection: neutralise cells that start like a formula.
  if (/^[=+\-@\t\r]/.test(s) && !isNum(v)) s = `'${s}`;
  return /[",\r\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

export function toCsv(columns: readonly string[], rows: readonly unknown[][]): string {
  return [columns, ...rows].map((r) => r.map(csvField).join(',')).join('\r\n') + '\r\n';
}

/** Offers `text` as a file download (browser and webview). */
export function downloadText(name: string, text: string, mime = 'text/csv'): void {
  const url = URL.createObjectURL(new Blob([text], { type: `${mime};charset=utf-8` }));
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url));
}
